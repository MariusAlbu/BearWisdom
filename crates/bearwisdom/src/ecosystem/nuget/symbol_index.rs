// =============================================================================
// nuget/symbol_index.rs — Supplementary `(module, name) → file` index over
// `.cs` source files in NuGet package roots. Used by the `Ecosystem` trait's
// demand-driven surface; the eager DLL pass populates rows independently.
// =============================================================================

use super::cs_header::scan_cs_header;
use super::source_discovery::discover_nuget_source_files;
use crate::ecosystem::externals::ExternalDepRoot;
use crate::ecosystem::SymbolLocationIndex;
use crate::walker::WalkedFile;

/// Build a `SymbolLocationIndex` from `.cs` source files in the given dep
/// roots. Called via the `Ecosystem::build_symbol_index` method when the
/// pipeline constructs synthetic NuGet dep roots. In the primary eager
/// indexing flow the supplementary source scan happens inside
/// `parse_dotnet_externals_with_source` instead.
pub(crate) fn build_nuget_source_symbol_index(
    dep_roots: &[ExternalDepRoot],
) -> SymbolLocationIndex {
    let mut index = SymbolLocationIndex::new();
    for dep in dep_roots {
        for src_path in discover_nuget_source_files(&dep.root) {
            let Ok(content) = std::fs::read_to_string(&src_path) else {
                continue;
            };
            for sym in scan_cs_header(&content) {
                index.insert(dep.module_path.clone(), sym.name, src_path.clone());
            }
        }
    }
    index
}

/// Return `WalkedFile` entries for `.cs` source files in `dep.root` that
/// declare any of the requested symbol short names.
pub(crate) fn resolve_nuget_source_symbols(
    dep: &ExternalDepRoot,
    symbols: &[&str],
) -> Vec<WalkedFile> {
    if symbols.is_empty() {
        return Vec::new();
    }
    let source_files = discover_nuget_source_files(&dep.root);
    if source_files.is_empty() {
        return Vec::new();
    }

    let targets: std::collections::HashSet<&str> = symbols.iter().copied().collect();
    let mut out = Vec::new();

    for src_path in source_files {
        let Ok(content) = std::fs::read_to_string(&src_path) else {
            continue;
        };
        let decls = scan_cs_header(&content);
        if decls.iter().any(|d| targets.contains(d.name.as_str())) {
            let rel = src_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push(WalkedFile {
                relative_path: format!("ext:dotnet-src:{}/{}", dep.module_path, rel),
                absolute_path: src_path,
                language: "csharp",
            });
        }
    }
    out
}
