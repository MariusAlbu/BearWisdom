// =============================================================================
// nuget/source_discovery.rs — find `.cs` source files inside resolved NuGet
// package version directories and parse them header-only into ParsedFiles.
//
// NuGet packages can ship source three ways: `contentFiles/cs/<tfm>/`,
// `lib/<tfm>/` (rare), or `src/`. Source-only packages (Microsoft.Bcl.*)
// place files at the package root.
// =============================================================================

use std::path::{Path, PathBuf};

use super::cs_header::scan_cs_header;
use super::dll_metadata::largest_subdir;

/// Discover `.cs` source files shipped inside a NuGet package version dir.
/// Checks in priority order:
///   1. `contentFiles/cs/<tfm>/**/*.cs` — NuGet contentFiles convention
///   2. `lib/<tfm>/**/*.cs` — rare but exists in some packages
///   3. `src/**/*.cs` — source-only packages (Microsoft.Bcl.*, etc.)
///   4. Top-level `*.cs` at the package root
///
/// Returns deduped absolute paths.
pub(crate) fn discover_nuget_source_files(version_dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    // 1. contentFiles/cs/<tfm>/
    let content_files_cs = version_dir.join("contentFiles").join("cs");
    if content_files_cs.is_dir() {
        let preferred_tfms = [
            "net9.0",
            "net8.0",
            "net7.0",
            "net6.0",
            "netstandard2.1",
            "netstandard2.0",
            "any",
        ];
        let tfm_dir = preferred_tfms
            .iter()
            .map(|tfm| content_files_cs.join(tfm))
            .find(|p| p.is_dir())
            .or_else(|| largest_subdir(&content_files_cs));
        if let Some(dir) = tfm_dir {
            collect_cs_files(&dir, &mut out, &mut seen, 0);
        }
    }

    // 2. lib/<tfm>/**/*.cs
    let lib_dir = version_dir.join("lib");
    if lib_dir.is_dir() {
        let preferred_tfms = [
            "net9.0",
            "net8.0",
            "net7.0",
            "net6.0",
            "netstandard2.1",
            "netstandard2.0",
        ];
        let tfm_dir = preferred_tfms
            .iter()
            .map(|tfm| lib_dir.join(tfm))
            .find(|p| p.is_dir())
            .or_else(|| largest_subdir(&lib_dir));
        if let Some(dir) = tfm_dir {
            collect_cs_files(&dir, &mut out, &mut seen, 0);
        }
    }

    // 3. src/
    let src_dir = version_dir.join("src");
    if src_dir.is_dir() {
        collect_cs_files(&src_dir, &mut out, &mut seen, 0);
    }

    // 4. Top-level *.cs
    if let Ok(entries) = std::fs::read_dir(version_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "cs") {
                if seen.insert(path.clone()) {
                    out.push(path);
                }
            }
        }
    }

    out
}

/// Recursive `.cs` collector with depth cap. Skips build-artifact and
/// test subdirectories.
fn collect_cs_files(
    dir: &Path,
    out: &mut Vec<PathBuf>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: usize,
) {
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    "obj" | "bin" | "test" | "tests" | "samples" | "examples" | ".git"
                ) {
                    continue;
                }
            }
            collect_cs_files(&path, out, seen, depth + 1);
        } else if ft.is_file() && path.extension().is_some_and(|e| e == "cs") {
            if seen.insert(path.clone()) {
                out.push(path);
            }
        }
    }
}

/// Parse a single `.cs` source file header-only, returning a synthetic
/// `ParsedFile`. Uses `ext:dotnet-src:<pkg_name>/<filename>` as the virtual
/// path so it's distinguishable from DLL-synthesized rows. Real line numbers
/// are preserved for chain walkers.
pub(crate) fn parse_cs_source_file(
    path: &Path,
    pkg_name: &str,
    lang_id: &str,
) -> std::result::Result<crate::types::ParsedFile, String> {
    use crate::types::{ExtractedSymbol, ParsedFile, Visibility};

    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let decls = scan_cs_header(&content);

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown.cs".to_string());
    let virtual_path = format!("ext:dotnet-src:{pkg_name}/{file_name}");

    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let size = metadata.len();
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    let content_hash = format!("{:x}", size);
    let line_count = content.lines().count() as u32;

    let extracted: Vec<ExtractedSymbol> = decls
        .into_iter()
        .map(|sym| ExtractedSymbol {
            name: sym.name.clone(),
            qualified_name: if sym.scope.is_empty() {
                sym.name.clone()
            } else {
                format!("{}.{}", sym.scope, sym.name)
            },
            kind: sym.kind,
            visibility: Some(Visibility::Public),
            start_line: sym.line as u32,
            end_line: sym.line as u32,
            start_col: 0,
            end_col: 0,
            signature: sym.signature,
            doc_comment: None,
            scope_path: if sym.scope.is_empty() {
                None
            } else {
                Some(sym.scope)
            },
            parent_index: None,
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        })
        .collect();

    Ok(ParsedFile {
        path: virtual_path,
        language: lang_id.to_string(),
        content_hash,
        size,
        line_count,
        mtime,
        package_id: None,
        symbols: extracted,
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
