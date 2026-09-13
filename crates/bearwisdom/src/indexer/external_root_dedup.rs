// =============================================================================
// indexer/external_root_dedup.rs — collapse discovered dep roots to one entry
// per physical module copy
// =============================================================================

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ecosystem::externals::ExternalDepRoot;

/// Resolve `path` to the identity it should dedup on: the OS-canonical form
/// when the path exists (so every symlink pointing at the same physical
/// directory collapses to one value), falling back to `path` itself when
/// canonicalization fails (broken symlink, permission error, or a path that
/// doesn't exist on disk — dedup then degrades to per-path identity).
///
/// Windows' `canonicalize` returns the `\\?\`-prefixed verbatim form, which
/// does not compare equal to (or `strip_prefix` against) the plain paths used
/// everywhere else in this codebase — stripped here so the key is a plain
/// path like every other `PathBuf` this module handles.
fn canonical_dedup_path(path: &Path) -> PathBuf {
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    match canon.to_str() {
        Some(s) => match s.strip_prefix(r"\\?\") {
            Some(stripped) => PathBuf::from(stripped),
            None => canon,
        },
        None => canon,
    }
}

/// Collapse `roots` to one entry per `(ecosystem, module_path, version,
/// canonical root path)`, returning each surviving root with the ids of every
/// workspace package that declared it.
///
/// Root path is part of the key so distinct package directories representing
/// the same module stay separate roots to walk. It is canonicalized for the
/// KEY ONLY — the stored `ExternalDepRoot` keeps its original, un-canonicalized
/// `root`, so relative-import resolution and virtual-path construction are
/// unaffected. A symlink-hoisted dependency is reachable through several
/// package paths; each is a distinct `PathBuf` resolving to one physical
/// directory, so without canonicalizing, N symlinked copies survive as N
/// separate roots — each with its own independent symbol-index scan, and
/// whichever scan runs first wins any first-writer-wins collision for a name
/// the package re-exports through more than one file.
pub(crate) fn dedup_roots(roots: Vec<ExternalDepRoot>) -> Vec<(ExternalDepRoot, Vec<i64>)> {
    let mut deduped: Vec<(ExternalDepRoot, Vec<i64>)> = Vec::new();
    let mut root_index: HashMap<(&'static str, String, String, PathBuf), usize> = HashMap::new();
    for root in roots {
        let key = (
            root.ecosystem,
            root.module_path.clone(),
            root.version.clone(),
            canonical_dedup_path(&root.root),
        );
        if let Some(&idx) = root_index.get(&key) {
            if let Some(pid) = root.package_id {
                if !deduped[idx].1.contains(&pid) {
                    deduped[idx].1.push(pid);
                }
            }
        } else {
            root_index.insert(key, deduped.len());
            let declaring = root.package_id.map(|p| vec![p]).unwrap_or_default();
            deduped.push((root, declaring));
        }
    }
    deduped
}

#[cfg(test)]
#[path = "external_root_dedup_tests.rs"]
mod tests;
