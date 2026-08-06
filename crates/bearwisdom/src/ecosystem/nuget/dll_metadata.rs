// =============================================================================
// nuget/dll_metadata.rs — ECMA-335 DLL → ParsedFile via `dotscope`.
//
// Synthesizes `ParsedFile`s of public types + methods from the DLLs
// `dll_locator` discovers: the eager whole-DLL pass (per-coord work on rayon),
// the cheap type-name enumeration the demand index offers, and the
// materialize-on-miss crack of one demanded type on a dedicated dotscope
// thread.
// =============================================================================

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use rayon::prelude::*;
use tracing::debug;

use super::dll_locator::{
    collect_dotnet_project_files, collect_transitive_coords_from_assets_json,
    collect_transitive_coords_from_deps_json, dominant_dotnet_language, find_dlls_in_version_dir,
    largest_version_subdir, nuget_packages_root,
};
use super::manifest::{parse_package_references_full, NuGetCoord};
use super::signature_format::{
    format_generic_suffix, format_method_signature, strip_backtick_arity,
};
use super::source_discovery::{discover_nuget_source_files, parse_cs_source_file};

/// A request to crack one type out of a DLL, sent to the dedicated dotscope
/// thread. `reply` carries the resulting `ParsedFile` (or `None`) back.
struct CrackRequest {
    dll_path: PathBuf,
    qualified_type: String,
    lang_id: String,
    virtual_path: String,
    reply: std::sync::mpsc::Sender<Option<crate::types::ParsedFile>>,
}

/// All `dotscope` work runs on ONE dedicated OS thread, never on the resolve
/// pool's rayon workers.
///
/// Two reasons this is mandatory: (1) `dotscope` is not safe to use from
/// multiple threads at once (parallel parse OR read deadlocks); (2) `dotscope`
/// itself uses rayon — calling it from a resolve-pool worker nests its parallel
/// work on the resolve pool, which deadlocks when the other workers are blocked
/// waiting on this DLL. Running on a plain (non-rayon) thread routes dotscope's
/// internal rayon to the idle global pool instead, and the single thread
/// serializes access. The per-DLL `CilObject` cache lives on this thread, so it
/// needs no lock and each DLL is parsed at most once.
static DOTSCOPE_TX: Lazy<Mutex<std::sync::mpsc::Sender<CrackRequest>>> = Lazy::new(|| {
    let (tx, rx) = std::sync::mpsc::channel::<CrackRequest>();
    std::thread::Builder::new()
        .name("bw-dotscope".into())
        .spawn(move || {
            let mut cache: HashMap<PathBuf, Option<Arc<dotscope::prelude::CilObject>>> =
                HashMap::new();
            while let Ok(req) = rx.recv() {
                let assembly = match cache.get(&req.dll_path) {
                    Some(entry) => entry.clone(),
                    None => {
                        let loaded = load_assembly(&req.dll_path).map(Arc::new);
                        cache.insert(req.dll_path.clone(), loaded.clone());
                        loaded
                    }
                };
                let result = assembly.and_then(|asm| {
                    extract_type_from_assembly(
                        &asm,
                        &req.qualified_type,
                        &req.lang_id,
                        &req.virtual_path,
                        &req.dll_path,
                    )
                });
                let _ = req.reply.send(result);
            }
        })
        .expect("failed to spawn dotscope worker thread");
    Mutex::new(tx)
});

/// Parse one DLL into a `CilObject`. Only ever called on the dotscope thread.
fn load_assembly(dll_path: &Path) -> Option<dotscope::prelude::CilObject> {
    use dotscope::metadata::cilassemblyview::CilAssemblyView;
    use dotscope::metadata::validation::ValidationConfig;
    use dotscope::prelude::CilObject;

    let mut config = ValidationConfig::disabled();
    config.lenient = true;
    let view = CilAssemblyView::from_path_with_validation(dll_path, config.clone()).ok()?;
    CilObject::from_view_with_validation(view, config).ok()
}

/// Public entry point used by back-compat re-exports in `externals.rs`.
/// Returns DLL metadata ParsedFiles only — source ParsedFiles are merged by
/// the `ExternalSourceLocator::parse_metadata_only` impl above.
pub fn parse_dotnet_externals(project_root: &Path) -> Vec<crate::types::ParsedFile> {
    let (dll_pf, _source_pf) = parse_dotnet_externals_with_source(project_root);
    dll_pf
}

/// Enumerate the public type names from a DLL without synthesizing full
/// ParsedFile entries for each. Returns `(simple_name, namespace, virtual_path)`
/// for every public/public-nested type. Cheap: only reads the type-definition
/// table header from the ECMA-335 metadata, not method bodies.
///
/// `virtual_path` encodes `"ext:dotnet-type:<dll_abs>!!<namespace>.<TypeName>"`
/// (using `!!` as separator because `!` cannot appear in filesystem paths on
/// either Windows or Unix).
pub(crate) fn list_dll_type_names(
    dll_path: &Path,
    package_name: &str,
) -> Vec<(String, String)> {
    use dotscope::metadata::cilassemblyview::CilAssemblyView;
    use dotscope::metadata::validation::ValidationConfig;
    use dotscope::prelude::CilObject;

    let mut config = ValidationConfig::disabled();
    config.lenient = true;
    let Ok(view) = CilAssemblyView::from_path_with_validation(dll_path, config.clone()) else {
        return Vec::new();
    };
    let Ok(assembly) = CilObject::from_view_with_validation(view, config) else {
        return Vec::new();
    };
    let assembly_name = assembly
        .assembly()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| package_name.to_string());
    let dll_str = dll_path.to_string_lossy().replace('\\', "/");
    let mut out = Vec::new();
    for type_def in assembly.types().all_types().iter() {
        let name = type_def.name.clone();
        let namespace = type_def.namespace.clone();
        if name.starts_with('<') || name == "<Module>" {
            continue;
        }
        let visibility_mask = type_def.flags & 0x07;
        if visibility_mask != 1 && visibility_mask != 2 {
            continue;
        }
        let simple = strip_backtick_arity(&name).to_string();
        let qualified = if namespace.is_empty() {
            simple.clone()
        } else {
            format!("{namespace}.{simple}")
        };
        // Virtual path encodes the DLL location + assembly name + qualified type name.
        // The materialize path decodes this to re-open just this one type.
        let virt = format!("ext:dotnet-type:{dll_str}!!{assembly_name}!!{qualified}");
        // A STATIC class's public static METHOD names are also offered: an
        // extension method is reached by METHOD name (`x.HasColumnType(...)`)
        // — no ref ever names the declaring `…Extensions` class, so without
        // these entries the class is never demanded. Materializing the entry
        // cracks the whole type, methods included.
        let is_static_class = type_def.flags & 0x80 != 0 && type_def.flags & 0x100 != 0;
        if is_static_class {
            for (_, method_ref) in type_def.methods.iter() {
                let Some(method) = method_ref.upgrade() else {
                    continue;
                };
                if method.name.starts_with('<') || method.name.starts_with('.') {
                    continue;
                }
                if method.flags_access
                    != dotscope::metadata::method::MethodAccessFlags::PUBLIC
                {
                    continue;
                }
                out.push((method.name.clone(), virt.clone()));
            }
        }
        out.push((simple, virt));
    }
    out
}

/// Crack a single .NET type from a DLL on demand. `virtual_path` must be in
/// the `ext:dotnet-type:<dll_path>!!<assembly_name>!!<qualified_type>` form
/// produced by `list_dll_type_names`. Returns `None` on any decoding or I/O
/// error so the caller can skip gracefully.
pub(crate) fn crack_one_dll_type(
    virtual_path: &str,
    lang_id: &str,
) -> Option<crate::types::ParsedFile> {
    // Decode "ext:dotnet-type:<dll_path>!!<assembly_name>!!<qualified_type>"
    let payload = virtual_path.strip_prefix("ext:dotnet-type:")?;
    let mut parts = payload.splitn(3, "!!");
    let dll_str = parts.next()?;
    let _assembly_name = parts.next()?;
    let qualified_type = parts.next()?;

    // Hand the crack to the dedicated dotscope thread and block for the reply —
    // dotscope must never run on a resolve-pool worker (see `DOTSCOPE_TX`).
    let (reply, reply_rx) = std::sync::mpsc::channel();
    let req = CrackRequest {
        dll_path: PathBuf::from(dll_str),
        qualified_type: qualified_type.to_string(),
        lang_id: lang_id.to_string(),
        virtual_path: virtual_path.to_string(),
        reply,
    };
    DOTSCOPE_TX.lock().ok()?.send(req).ok()?;
    reply_rx.recv().ok()?
}

/// Extract one type (and its public methods) from an already-parsed assembly.
/// Runs ONLY on the dotscope thread (see `DOTSCOPE_TX`).
fn extract_type_from_assembly(
    assembly: &dotscope::prelude::CilObject,
    qualified_type: &str,
    lang_id: &str,
    virtual_path: &str,
    dll_path: &Path,
) -> Option<crate::types::ParsedFile> {
    use dotscope::metadata::method::MethodAccessFlags;
    use crate::types::{ExtractedSymbol, SymbolKind};

    // Find the matching type definition by qualified name.
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    for type_def in assembly.types().all_types().iter() {
        let name = type_def.name.clone();
        let namespace = type_def.namespace.clone();
        if name.starts_with('<') || name == "<Module>" {
            continue;
        }
        let visibility_mask = type_def.flags & 0x07;
        if visibility_mask != 1 && visibility_mask != 2 {
            continue;
        }
        let display_name = strip_backtick_arity(&name);
        let this_qualified = if namespace.is_empty() {
            display_name.to_string()
        } else {
            format!("{namespace}.{display_name}")
        };
        if this_qualified != qualified_type {
            continue;
        }
        // ECMA-335 TypeAttributes: Abstract|Sealed together = a STATIC class,
        // the only container the C# compiler allows extension methods in.
        let is_static_class = type_def.flags & 0x80 != 0 && type_def.flags & 0x100 != 0;
        let is_interface = type_def.flags & 0x20 != 0;
        let kind = if is_interface {
            SymbolKind::Interface
        } else {
            SymbolKind::Class
        };
        let type_generic_names: Vec<String> = type_def
            .generic_params
            .iter()
            .map(|(_, gp)| gp.name.clone())
            .collect();
        let type_gp_suffix = format_generic_suffix(&type_generic_names);
        symbols.push(ExtractedSymbol {
            name: display_name.to_string(),
            qualified_name: qualified_type.to_string(),
            kind,
            visibility: Some(crate::types::Visibility::Public),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: Some(format!(
                "{} {}{}",
                if is_interface { "interface" } else { "class" },
                display_name,
                type_gp_suffix
            )),
            doc_comment: None,
            scope_path: if namespace.is_empty() {
                None
            } else {
                Some(namespace.clone())
            },
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        });
        for (_, method_ref) in type_def.methods.iter() {
            let Some(method) = method_ref.upgrade() else {
                continue;
            };
            if method.name.starts_with('<') || method.name.starts_with('.') {
                continue;
            }
            if method.flags_access != MethodAccessFlags::PUBLIC {
                continue;
            }
            let method_name = method.name.clone();
            let method_qname = format!("{qualified_type}.{method_name}");
            let method_generic_names: Vec<String> = method
                .generic_params
                .iter()
                .map(|(_, gp)| gp.name.clone())
                .collect();
            let is_extension_candidate = is_static_class
                && method
                    .flags_modifiers
                    .contains(dotscope::metadata::method::MethodModifiers::STATIC)
                && !method.signature.params.is_empty();
            let signature = format_method_signature(
                &method_name,
                &method.signature,
                &type_generic_names,
                &method_generic_names,
                &assembly,
                is_extension_candidate,
            );
            symbols.push(ExtractedSymbol {
                name: method_name,
                qualified_name: method_qname,
                kind: SymbolKind::Method,
                visibility: Some(crate::types::Visibility::Public),
                start_line: 0,
                end_line: 0,
                start_col: 0,
                end_col: 0,
                signature: Some(signature),
                doc_comment: None,
                scope_path: Some(qualified_type.to_string()),
                parent_index: None,
                byte_offset: 0,
                declared_type: None,
                return_type: None,
                param_types: Vec::new(),
                generic_params: Vec::new(),
            });
        }
        break;
    }
    if symbols.is_empty() {
        return None;
    }
    let metadata = std::fs::metadata(dll_path).ok();
    let size = metadata.as_ref().map_or(0, |m| m.len());
    let mtime = metadata
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    Some(crate::types::ParsedFile {
        path: virtual_path.to_string(),
        language: lang_id.to_string(),
        content_hash: format!("{:x}", size),
        size,
        line_count: 0,
        mtime,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    })
}

/// Internal: returns `(dll_parsed_files, source_parsed_files)`. Called by the
/// `ExternalSourceLocator::parse_metadata_only` impl which concatenates both.
/// Keeping them separate lets back-compat callers stay cheap (DLL-only).
pub(crate) fn parse_dotnet_externals_with_source(
    project_root: &Path,
) -> (Vec<crate::types::ParsedFile>, Vec<crate::types::ParsedFile>) {
    let mut project_files: Vec<PathBuf> = Vec::new();
    collect_dotnet_project_files(project_root, &mut project_files, 0);
    if project_files.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let mut coords: Vec<NuGetCoord> = Vec::new();
    for p in &project_files {
        let Ok(content) = std::fs::read_to_string(p) else {
            continue;
        };
        coords.extend(parse_package_references_full(&content));
    }

    for p in &project_files {
        if let Some(proj_dir) = p.parent() {
            coords.extend(collect_transitive_coords_from_deps_json(proj_dir));
            coords.extend(collect_transitive_coords_from_assets_json(proj_dir));
        }
    }

    if coords.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let Some(nuget_root) = nuget_packages_root() else {
        debug!("No NuGet packages cache; skipping .NET externals");
        return (Vec::new(), Vec::new());
    };
    debug!(
        "Probing NuGet cache {} for {} package references",
        nuget_root.display(),
        coords.len()
    );

    let lang_id = dominant_dotnet_language(&project_files);

    // Per-coord work runs in parallel — DLL metadata reads and per-file
    // source parses are I/O + CPU bound and independent across packages.
    // Dedupe (seen_dll / seen_src) is done single-threaded after the
    // parallel pass so Vec ordering stays deterministic. On a big .NET
    // solution (2000+ transitives) this is the dominant externals cost
    // and a near-linear win per available core.
    struct CoordResult {
        dlls: Vec<(PathBuf, crate::types::ParsedFile)>,
        srcs: Vec<(PathBuf, crate::types::ParsedFile)>,
    }

    let per_coord: Vec<CoordResult> = coords
        .par_iter()
        .map(|coord| {
            let pkg_dir = nuget_root.join(coord.name.to_lowercase());
            if !pkg_dir.is_dir() {
                return CoordResult {
                    dlls: Vec::new(),
                    srcs: Vec::new(),
                };
            }

            let version = if let Some(v) = &coord.version {
                let concrete = pkg_dir.join(v);
                if concrete.is_dir() {
                    v.clone()
                } else {
                    match largest_version_subdir(&pkg_dir) {
                        Some(v) => v,
                        None => {
                            return CoordResult {
                                dlls: Vec::new(),
                                srcs: Vec::new(),
                            }
                        }
                    }
                }
            } else {
                match largest_version_subdir(&pkg_dir) {
                    Some(v) => v,
                    None => {
                        return CoordResult {
                            dlls: Vec::new(),
                            srcs: Vec::new(),
                        }
                    }
                }
            };
            let version_dir = pkg_dir.join(&version);

            let dlls: Vec<(PathBuf, crate::types::ParsedFile)> =
                find_dlls_in_version_dir(&version_dir, &coord.name)
                    .into_iter()
                    .filter_map(|dll_path| {
                        match parse_dotnet_dll(&dll_path, &coord.name, lang_id) {
                            Ok(pf) => Some((dll_path, pf)),
                            Err(e) => {
                                debug!("Failed .NET metadata read {}: {e}", dll_path.display());
                                None
                            }
                        }
                    })
                    .collect();

            let mut srcs = Vec::new();
            for src_path in discover_nuget_source_files(&version_dir) {
                match parse_cs_source_file(&src_path, &coord.name, lang_id) {
                    Ok(pf) => srcs.push((src_path, pf)),
                    Err(e) => debug!("NuGet source parse error {}: {e}", src_path.display()),
                }
            }

            CoordResult { dlls, srcs }
        })
        .collect();

    let mut dll_out = Vec::new();
    let mut src_out = Vec::new();
    let mut seen_dll: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut seen_src: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for res in per_coord {
        for (path, pf) in res.dlls {
            if seen_dll.insert(path) {
                dll_out.push(pf);
            }
        }
        for (path, pf) in res.srcs {
            if seen_src.insert(path) {
                debug!(
                    "NuGet source: {} symbols from {}",
                    pf.symbols.len(),
                    pf.path
                );
                src_out.push(pf);
            }
        }
    }

    if !src_out.is_empty() {
        debug!(
            "NuGet hybrid: {} DLL + {} source-file entries for {}",
            dll_out.len(),
            src_out.len(),
            project_root.display()
        );
    }

    (dll_out, src_out)
}

/// Public shim so the DotnetStdlib ecosystem can reuse this DLL→ParsedFile
/// synthesizer for .NET reference assemblies. Identical contract to the
/// private helper.
pub(crate) fn parse_dotnet_dll_public(
    dll_path: &Path,
    package_name: &str,
    lang_id: &str,
) -> std::result::Result<crate::types::ParsedFile, String> {
    parse_dotnet_dll(dll_path, package_name, lang_id)
}

fn parse_dotnet_dll(
    dll_path: &Path,
    package_name: &str,
    lang_id: &str,
) -> std::result::Result<crate::types::ParsedFile, String> {
    use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind};
    use dotscope::metadata::cilassemblyview::CilAssemblyView;
    use dotscope::metadata::method::MethodAccessFlags;
    use dotscope::metadata::validation::ValidationConfig;
    use dotscope::prelude::CilObject;

    // BCL runtime DLLs (especially `System.Private.CoreLib.dll`) carry
    // compiler-internal signature shapes that dotscope's strict validators
    // reject — and `System.Exception`, `System.Object`, and the rest of the
    // root BCL types live in CoreLib. We only need type/method names +
    // signatures for symbol discovery, not full ECMA-335 conformance, so
    // disable validation entirely. Note: dotscope's
    // `CilObject::from_path_with_validation` always calls
    // `CilAssemblyView::from_path` internally, which uses
    // `ValidationConfig::production()` regardless of the config we pass —
    // Stage 1 (raw) validation runs there. We sidestep that by building the
    // view explicitly with `disabled()` and constructing the object via
    // `from_view_with_validation`. This skips both Stage 1 and Stage 2.
    // Custom config: every validator off + lenient: true. `disabled()` alone
    // sets validators off but leaves lenient = false, so the data loader
    // (`CilObjectData::from_assembly_view`) propagates parser errors instead
    // of treating them as warnings — that path catches CoreLib's custom
    // attributes with newer CIL element types dotscope hasn't implemented.
    let mut config = ValidationConfig::disabled();
    config.lenient = true;
    let view = CilAssemblyView::from_path_with_validation(dll_path, config.clone())
        .map_err(|e| e.to_string())?;
    let assembly = CilObject::from_view_with_validation(view, config).map_err(|e| e.to_string())?;
    let assembly_name = assembly
        .assembly()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| package_name.to_string());
    let virtual_path = format!("ext:dotnet:{}/{}", package_name, assembly_name);
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();

    for type_def in assembly.types().all_types().iter() {
        let name = type_def.name.clone();
        let namespace = type_def.namespace.clone();
        if name.starts_with('<') || name == "<Module>" {
            continue;
        }
        let visibility_mask = type_def.flags & 0x07;
        if visibility_mask != 1 && visibility_mask != 2 {
            continue;
        }
        // ECMA-335 TypeAttributes: Abstract|Sealed together = a STATIC class,
        // the only container the C# compiler allows extension methods in.
        let is_static_class = type_def.flags & 0x80 != 0 && type_def.flags & 0x100 != 0;
        let is_interface = type_def.flags & 0x20 != 0;
        let kind = if is_interface {
            SymbolKind::Interface
        } else {
            SymbolKind::Class
        };

        let display_name = strip_backtick_arity(&name);
        let qualified_name = if namespace.is_empty() {
            display_name.to_string()
        } else {
            format!("{namespace}.{display_name}")
        };

        let type_generic_names: Vec<String> = type_def
            .generic_params
            .iter()
            .map(|(_, gp)| gp.name.clone())
            .collect();
        let type_gp_suffix = format_generic_suffix(&type_generic_names);

        symbols.push(ExtractedSymbol {
            name: display_name.to_string(),
            qualified_name: qualified_name.clone(),
            kind,
            visibility: Some(crate::types::Visibility::Public),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: Some(format!(
                "{} {}{}",
                if is_interface { "interface" } else { "class" },
                display_name,
                type_gp_suffix
            )),
            doc_comment: None,
            scope_path: if namespace.is_empty() {
                None
            } else {
                Some(namespace.clone())
            },
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        });

        for (_, method_ref) in type_def.methods.iter() {
            let Some(method) = method_ref.upgrade() else {
                continue;
            };
            if method.name.starts_with('<') || method.name.starts_with('.') {
                continue;
            }
            if method.flags_access != MethodAccessFlags::PUBLIC {
                continue;
            }

            let method_name = method.name.clone();
            let method_qname = format!("{qualified_name}.{method_name}");
            let method_generic_names: Vec<String> = method
                .generic_params
                .iter()
                .map(|(_, gp)| gp.name.clone())
                .collect();
            let is_extension_candidate = is_static_class
                && method
                    .flags_modifiers
                    .contains(dotscope::metadata::method::MethodModifiers::STATIC)
                && !method.signature.params.is_empty();
            let signature = format_method_signature(
                &method_name,
                &method.signature,
                &type_generic_names,
                &method_generic_names,
                &assembly,
                is_extension_candidate,
            );
            symbols.push(ExtractedSymbol {
                name: method_name,
                qualified_name: method_qname,
                kind: SymbolKind::Method,
                visibility: Some(crate::types::Visibility::Public),
                start_line: 0,
                end_line: 0,
                start_col: 0,
                end_col: 0,
                signature: Some(signature),
                doc_comment: None,
                scope_path: Some(qualified_name.clone()),
                parent_index: None,
                byte_offset: 0,
                declared_type: None,
                return_type: None,
                param_types: Vec::new(),
                generic_params: Vec::new(),
            });
        }
    }

    debug!(
        "Parsed {} .NET symbols from {}",
        symbols.len(),
        dll_path.display()
    );

    let metadata = std::fs::metadata(dll_path).map_err(|e| e.to_string())?;
    let size = metadata.len();
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    let content_hash = format!("{:x}", size).to_string();

    Ok(ParsedFile {
        path: virtual_path,
        language: lang_id.to_string(),
        content_hash,
        size,
        line_count: 0,
        mtime,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    })
}



