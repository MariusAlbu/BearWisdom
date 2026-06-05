// =============================================================================
// nuget/dll_metadata.rs — NuGet cache discovery + ECMA-335 DLL → ParsedFile.
//
// The eager pass for .NET externals: walk every `*.csproj` / `.fsproj` /
// `.vbproj` in the project, collect PackageReference + transitive deps from
// `*.deps.json`, locate each package's preferred-TFM DLL under
// `~/.nuget/packages/`, and synthesize a `ParsedFile` of public types +
// methods via `dotscope`. Per-coord work runs on rayon.
// =============================================================================

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use tracing::debug;

use super::manifest::{parse_package_references_full, NuGetCoord};
use super::source_discovery::{discover_nuget_source_files, parse_cs_source_file};

/// Public entry point used by back-compat re-exports in `externals.rs`.
/// Returns DLL metadata ParsedFiles only — source ParsedFiles are merged by
/// the `ExternalSourceLocator::parse_metadata_only` impl above.
pub fn parse_dotnet_externals(project_root: &Path) -> Vec<crate::types::ParsedFile> {
    let (dll_pf, _source_pf) = parse_dotnet_externals_with_source(project_root);
    dll_pf
}

/// Internal: returns `(dll_parsed_files, source_parsed_files)`. Called by the
/// `ExternalSourceLocator::parse_metadata_only` impl which concatenates both.
/// Keeping them separate lets back-compat callers stay cheap (DLL-only).
pub(crate) fn parse_dotnet_externals_with_source(
    project_root: &Path,
) -> (Vec<crate::types::ParsedFile>, Vec<crate::types::ParsedFile>) {
    let mut project_files: Vec<PathBuf> = Vec::new();
    collect_dotnet_project_files(project_root, &mut project_files, 0);
    if project_files.is_empty() { return (Vec::new(), Vec::new()) }

    let mut coords: Vec<NuGetCoord> = Vec::new();
    for p in &project_files {
        let Ok(content) = std::fs::read_to_string(p) else { continue };
        coords.extend(parse_package_references_full(&content));
    }

    for p in &project_files {
        if let Some(proj_dir) = p.parent() {
            coords.extend(collect_transitive_coords_from_deps_json(proj_dir));
        }
    }

    if coords.is_empty() { return (Vec::new(), Vec::new()) }

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
        dll: Option<(PathBuf, crate::types::ParsedFile)>,
        srcs: Vec<(PathBuf, crate::types::ParsedFile)>,
    }

    let per_coord: Vec<CoordResult> = coords
        .par_iter()
        .map(|coord| {
            let pkg_dir = nuget_root.join(coord.name.to_lowercase());
            if !pkg_dir.is_dir() {
                return CoordResult { dll: None, srcs: Vec::new() };
            }

            let version = if let Some(v) = &coord.version {
                let concrete = pkg_dir.join(v);
                if concrete.is_dir() { v.clone() }
                else {
                    match largest_version_subdir(&pkg_dir) {
                        Some(v) => v,
                        None => return CoordResult { dll: None, srcs: Vec::new() },
                    }
                }
            } else {
                match largest_version_subdir(&pkg_dir) {
                    Some(v) => v,
                    None => return CoordResult { dll: None, srcs: Vec::new() },
                }
            };
            let version_dir = pkg_dir.join(&version);

            let dll = find_dll_in_version_dir(&version_dir, &coord.name)
                .and_then(|dll_path| match parse_dotnet_dll(&dll_path, &coord.name, lang_id) {
                    Ok(pf) => Some((dll_path, pf)),
                    Err(e) => {
                        debug!("Failed .NET metadata read {}: {e}", dll_path.display());
                        None
                    }
                });

            let mut srcs = Vec::new();
            for src_path in discover_nuget_source_files(&version_dir) {
                match parse_cs_source_file(&src_path, &coord.name, lang_id) {
                    Ok(pf) => srcs.push((src_path, pf)),
                    Err(e) => debug!("NuGet source parse error {}: {e}", src_path.display()),
                }
            }

            CoordResult { dll, srcs }
        })
        .collect();

    let mut dll_out = Vec::new();
    let mut src_out = Vec::new();
    let mut seen_dll: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut seen_src: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for res in per_coord {
        if let Some((path, pf)) = res.dll {
            if seen_dll.insert(path) { dll_out.push(pf); }
        }
        for (path, pf) in res.srcs {
            if seen_src.insert(path) {
                debug!("NuGet source: {} symbols from {}", pf.symbols.len(), pf.path);
                src_out.push(pf);
            }
        }
    }

    if !src_out.is_empty() {
        debug!(
            "NuGet hybrid: {} DLL + {} source-file entries for {}",
            dll_out.len(), src_out.len(), project_root.display()
        );
    }

    (dll_out, src_out)
}

fn collect_transitive_coords_from_deps_json(proj_dir: &Path) -> Vec<NuGetCoord> {
    let mut deps_json_files: Vec<PathBuf> = Vec::new();
    collect_deps_json(&proj_dir.join("bin"), &mut deps_json_files, 0);
    if deps_json_files.is_empty() { return Vec::new() }

    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for path in deps_json_files.iter().take(16) {
        let Ok(content) = std::fs::read_to_string(path) else { continue };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else { continue };
        let Some(libs) = json.get("libraries").and_then(|v| v.as_object()) else { continue };
        for (key, value) in libs {
            let ty = value.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if ty != "package" { continue }
            let Some((name, version)) = key.rsplit_once('/') else { continue };
            if !seen.insert(key.clone()) { continue }
            out.push(NuGetCoord { name: name.to_string(), version: Some(version.to_string()) });
        }
    }
    out
}

fn collect_deps_json(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 5 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(name, "obj" | "runtimes" | "ref") { continue }
                }
                collect_deps_json(&path, out, depth + 1);
            } else if ft.is_file()
                && path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".deps.json"))
            {
                out.push(path);
            }
        }
    }
}

fn dominant_dotnet_language(project_files: &[PathBuf]) -> &'static str {
    let mut cs = 0usize;
    let mut fs = 0usize;
    let mut vb = 0usize;
    for p in project_files {
        match p.extension().and_then(|e| e.to_str()) {
            Some("csproj") => cs += 1,
            Some("fsproj") => fs += 1,
            Some("vbproj") => vb += 1,
            _ => {}
        }
    }
    if cs >= fs && cs >= vb { "csharp" }
    else if fs >= vb { "fsharp" }
    else { "vbnet" }
}

pub fn nuget_packages_root() -> Option<PathBuf> {
    for key in ["BEARWISDOM_NUGET_PACKAGES", "NUGET_PACKAGES"] {
        if let Some(raw) = std::env::var_os(key) {
            let p = PathBuf::from(raw);
            if p.is_dir() { return Some(p) }
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let candidate = PathBuf::from(home).join(".nuget").join("packages");
    if candidate.is_dir() { Some(candidate) } else { None }
}

/// Locate the `.dll` matching `pkg_name` inside an already-resolved
/// `<nuget-cache>/<pkg-id>/<version>/` directory. Returns `None` for
/// source-only packages that ship no `lib/` directory.
fn find_dll_in_version_dir(version_dir: &Path, pkg_name: &str) -> Option<PathBuf> {
    let lib_dir = version_dir.join("lib");
    if !lib_dir.is_dir() { return None }

    let preferred_tfms = ["net9.0", "net8.0", "net7.0", "net6.0", "netstandard2.1", "netstandard2.0"];
    let mut chosen_tfm: Option<PathBuf> = None;
    for tfm in preferred_tfms {
        let candidate = lib_dir.join(tfm);
        if candidate.is_dir() { chosen_tfm = Some(candidate); break }
    }
    let tfm_dir = chosen_tfm.or_else(|| largest_subdir(&lib_dir))?;

    let entries = std::fs::read_dir(&tfm_dir).ok()?;
    let target_lower = pkg_name.to_lowercase() + ".dll";
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name == target_lower { return Some(entry.path()) }
    }
    None
}

fn largest_version_subdir(dir: &Path) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut versions: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            if e.file_type().ok()?.is_dir() { e.file_name().into_string().ok() } else { None }
        })
        .collect();
    versions.sort();
    versions.into_iter().next_back()
}

pub(crate) fn largest_subdir(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subs: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            if e.file_type().ok()?.is_dir() { Some(e.path()) } else { None }
        })
        .collect();
    subs.sort();
    subs.into_iter().next_back()
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
    let assembly = CilObject::from_view_with_validation(view, config)
        .map_err(|e| e.to_string())?;
    let assembly_name = assembly
        .assembly()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| package_name.to_string());
    let virtual_path = format!("ext:dotnet:{}/{}", package_name, assembly_name);
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();

    for type_def in assembly.types().all_types().iter() {
        let name = type_def.name.clone();
        let namespace = type_def.namespace.clone();
        if name.starts_with('<') || name == "<Module>" { continue }
        let visibility_mask = type_def.flags & 0x07;
        if visibility_mask != 1 && visibility_mask != 2 { continue }
        let is_interface = type_def.flags & 0x20 != 0;
        let kind = if is_interface { SymbolKind::Interface } else { SymbolKind::Class };

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
            start_line: 0, end_line: 0, start_col: 0, end_col: 0,
            signature: Some(format!(
                "{} {}{}",
                if is_interface { "interface" } else { "class" },
                display_name,
                type_gp_suffix
            )),
            doc_comment: None,
            scope_path: if namespace.is_empty() { None } else { Some(namespace.clone()) },
            parent_index: None,
            byte_offset: 0,
                    declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
});

        for (_, method_ref) in type_def.methods.iter() {
            let Some(method) = method_ref.upgrade() else { continue };
            if method.name.starts_with('<') || method.name.starts_with('.') { continue }
            if method.flags_access != MethodAccessFlags::PUBLIC { continue }

            let method_name = method.name.clone();
            let method_qname = format!("{qualified_name}.{method_name}");
            let method_generic_names: Vec<String> = method
                .generic_params
                .iter()
                .map(|(_, gp)| gp.name.clone())
                .collect();
            let signature = format_method_signature(
                &method_name,
                &method.signature,
                &type_generic_names,
                &method_generic_names,
                &assembly,
            );
            symbols.push(ExtractedSymbol {
                name: method_name,
                qualified_name: method_qname,
                kind: SymbolKind::Method,
                visibility: Some(crate::types::Visibility::Public),
                start_line: 0, end_line: 0, start_col: 0, end_col: 0,
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

    debug!("Parsed {} .NET symbols from {}", symbols.len(), dll_path.display());

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

pub(crate) fn strip_backtick_arity(name: &str) -> &str {
    match name.find('`') { Some(idx) => &name[..idx], None => name }
}

pub(crate) fn format_generic_suffix(names: &[String]) -> String {
    if names.is_empty() { String::new() } else { format!("<{}>", names.join(", ")) }
}

fn format_method_signature(
    method_name: &str,
    sig: &dotscope::metadata::signatures::SignatureMethod,
    type_generic_names: &[String],
    method_generic_names: &[String],
    assembly: &dotscope::prelude::CilObject,
) -> String {
    let gp_suffix = format_generic_suffix(method_generic_names);
    let mut params_str = String::from("(");
    for (i, p) in sig.params.iter().enumerate() {
        if i > 0 { params_str.push_str(", "); }
        let rendered = format!("{}", p);
        let substituted = substitute_generic_placeholders(&rendered, type_generic_names, method_generic_names);
        params_str.push_str(&resolve_signature_tokens(&substituted, assembly));
    }
    params_str.push(')');
    let return_rendered = format!("{}", sig.return_type);
    let return_substituted = substitute_generic_placeholders(&return_rendered, type_generic_names, method_generic_names);
    let return_str = resolve_signature_tokens(&return_substituted, assembly);
    format!("{method_name}{gp_suffix}{params_str}: {return_str}")
}

fn resolve_signature_tokens(
    rendered: &str,
    assembly: &dotscope::prelude::CilObject,
) -> String {
    use dotscope::metadata::token::Token;
    let type_registry = assembly.types();
    let imports = assembly.imports().cil();

    let mut out = String::with_capacity(rendered.len());
    let bytes = rendered.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let remaining = &rendered[i..];
        let (prefix_len, skip_prefix) = if remaining.starts_with("class[") {
            (6, true)
        } else if remaining.starts_with("valuetype[") {
            (10, true)
        } else {
            (0, false)
        };
        if skip_prefix {
            let after_prefix = &remaining[prefix_len..];
            if let Some(close_rel) = after_prefix.find(']') {
                let hex = &after_prefix[..close_rel];
                if let Ok(value) = u32::from_str_radix(hex, 16) {
                    let token = Token::new(value);
                    let table_byte = value >> 24;
                    let resolved: Option<String> = match table_byte {
                        0x02 => type_registry.get(&token).map(|ty| {
                            let name = strip_backtick_arity(&ty.name).to_string();
                            if ty.namespace.is_empty() { name } else { format!("{}.{}", ty.namespace, name) }
                        }),
                        0x01 => imports.get(token).map(|imp| {
                            let name = strip_backtick_arity(&imp.name).to_string();
                            if imp.namespace.is_empty() { name } else { format!("{}.{}", imp.namespace, name) }
                        }),
                        _ => None,
                    };
                    if let Some(full) = resolved {
                        out.push_str(&full);
                        i += prefix_len + close_rel + 1;
                        continue;
                    }
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

pub(crate) fn substitute_generic_placeholders(
    rendered: &str,
    type_gen: &[String],
    method_gen: &[String],
) -> String {
    let bytes = rendered.as_bytes();
    let mut out = String::with_capacity(rendered.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'!' {
            let is_method = i + 1 < bytes.len() && bytes[i + 1] == b'!';
            let num_start = if is_method { i + 2 } else { i + 1 };
            let mut num_end = num_start;
            while num_end < bytes.len() && bytes[num_end].is_ascii_digit() { num_end += 1 }
            if num_end > num_start {
                let idx: usize = rendered[num_start..num_end].parse().unwrap_or(usize::MAX);
                let target = if is_method { method_gen } else { type_gen };
                if let Some(name) = target.get(idx) {
                    out.push_str(name);
                    i = num_end;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn collect_dotnet_project_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 10 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(
                        name,
                        "bin" | "obj" | "node_modules" | ".git" | "target"
                            | "packages" | ".vs" | "TestResults" | "artifacts"
                    ) { continue }
                }
                collect_dotnet_project_files(&path, out, depth + 1);
            } else if ft.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if matches!(ext, "csproj" | "fsproj" | "vbproj") { out.push(path) }
                }
            }
        }
    }
}
