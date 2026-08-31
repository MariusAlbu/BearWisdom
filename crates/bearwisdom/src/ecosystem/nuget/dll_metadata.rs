// =============================================================================
// nuget/dll_metadata.rs — ECMA-335 DLL → ParsedFile via `dotscope`.
//
// Synthesizes `ParsedFile`s of public types + methods from the DLLs
// `dll_locator` discovers: the eager whole-DLL pass (per-coord work on rayon),
// the cheap type-name enumeration the demand index offers, and the extraction
// of one demanded type from an assembly `dotscope_worker` parsed.
// =============================================================================

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use tracing::debug;

use super::clr_projection::SourceNameProjection;
use super::dll_locator::{
    collect_dotnet_project_files, collect_transitive_coords_from_assets_json,
    collect_transitive_coords_from_deps_json, dominant_dotnet_language, find_dlls_in_version_dir,
    nuget_packages_root,
};
use super::manifest::{parse_package_references_full, NuGetCoord};
use super::source_discovery::{discover_nuget_source_files, parse_cs_source_file};
use super::type_qname::assembly_type_defs;
use super::type_symbols::{
    emit_type_symbols, is_public_type, is_static_class, projected_type_identity,
};
use super::version_select::select_version_subdir;

/// Public entry point used by back-compat re-exports in `externals.rs`.
/// Returns DLL metadata ParsedFiles only — source ParsedFiles are merged by
/// the `ExternalSourceLocator::parse_metadata_only` impl above.
pub fn parse_dotnet_externals(project_root: &Path) -> Vec<crate::types::ParsedFile> {
    let (dll_pf, _source_pf) = parse_dotnet_externals_with_source(project_root);
    dll_pf
}

/// Enumerate the public type names from a DLL without synthesizing full
/// ParsedFile entries for each. Returns `(lookup_name, virtual_path)` for every
/// public/public-nested type — its simple name, plus each public method name of
/// a static class, since an extension method is reached by METHOD name and no
/// ref ever names the declaring class. Cheap: only reads the type-definition
/// table header from the ECMA-335 metadata, not method bodies.
///
/// `virtual_path` encodes
/// `"ext:dotnet-type:<dll_abs>!!<assembly_name>!!<QualifiedTypeName>"` (using
/// `!!` as separator because `!` cannot appear in filesystem paths on either
/// Windows or Unix).
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
    let projection = SourceNameProjection::from_assembly(&assembly);
    let dll_str = dll_path.to_string_lossy().replace('\\', "/");
    let mut out = Vec::new();
    for type_def in assembly_type_defs(&assembly).iter() {
        let name = type_def.name.clone();
        if name.starts_with('<') || name == "<Module>" {
            continue;
        }
        if !is_public_type(type_def) {
            continue;
        }
        let (display, qualified, _) = projected_type_identity(type_def, &projection);
        // Virtual path encodes the DLL location + assembly name + qualified type name.
        // The materialize path decodes this to re-open just this one type.
        let virt = format!("ext:dotnet-type:{dll_str}!!{assembly_name}!!{qualified}");
        // A STATIC class's (or compiled module's) public static METHOD names
        // are also offered under their source-projected form: an extension
        // method or module function is reached by MEMBER name — no ref ever
        // names the declaring `…Extensions`/`…Module` class — so without
        // these entries the type is never demanded. Materializing the entry
        // cracks the whole type, methods included.
        if is_static_class(type_def) || projection.is_module(type_def.token.value()) {
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
                let offered = projection
                    .method_name(method.token.value(), &method.name)
                    .to_string();
                out.push((offered, virt.clone()));
            }
        }
        out.push((display, virt));
    }
    out
}

/// Extract one type (and its public members) from an already-parsed assembly.
/// Runs ONLY on the dotscope thread (see `dotscope_worker`).
pub(super) fn extract_type_from_assembly(
    assembly: &dotscope::prelude::CilObject,
    projection: &SourceNameProjection,
    qualified_type: &str,
    lang_id: &str,
    virtual_path: &str,
    dll_path: &Path,
) -> Option<crate::types::ParsedFile> {
    // Find the matching type definition by projected qualified name — the
    // same identity `list_dll_type_names` minted into the virtual path.
    let mut symbols: Vec<crate::types::ExtractedSymbol> = Vec::new();
    let mut refs: Vec<crate::types::ExtractedRef> = Vec::new();
    for type_def in assembly_type_defs(assembly).iter() {
        let name = type_def.name.clone();
        if name.starts_with('<') || name == "<Module>" {
            continue;
        }
        if !is_public_type(type_def) {
            continue;
        }
        let (_, this_qualified, _) = projected_type_identity(type_def, projection);
        if this_qualified != qualified_type {
            continue;
        }
        emit_type_symbols(type_def, assembly, projection, &mut symbols, &mut refs);
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
        refs,
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
        declared_modules: Vec::new(),
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

            let version = match select_version_subdir(&pkg_dir, coord.version.as_deref()) {
                Some(v) => v,
                None => {
                    return CoordResult {
                        dlls: Vec::new(),
                        srcs: Vec::new(),
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

pub(super) fn parse_dotnet_dll(
    dll_path: &Path,
    package_name: &str,
    lang_id: &str,
) -> std::result::Result<crate::types::ParsedFile, String> {
    use crate::types::{ExtractedSymbol, ParsedFile};
    use dotscope::metadata::cilassemblyview::CilAssemblyView;
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
    let projection = SourceNameProjection::from_assembly(&assembly);
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<crate::types::ExtractedRef> = Vec::new();

    for type_def in assembly_type_defs(&assembly).iter() {
        let name = type_def.name.clone();
        if name.starts_with('<') || name == "<Module>" {
            continue;
        }
        if !is_public_type(type_def) {
            continue;
        }
        emit_type_symbols(type_def, &assembly, &projection, &mut symbols, &mut refs);
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
        refs,
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
        declared_modules: Vec::new(),
    })
}



