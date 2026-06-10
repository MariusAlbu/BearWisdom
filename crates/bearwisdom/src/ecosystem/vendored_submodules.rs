// =============================================================================
// ecosystem/vendored_submodules.rs — in-tree vendored dependency discovery
//
// Reads `.gitmodules` to locate git-submodule subtrees. A submodule is a
// vendored third-party dependency the project declares (path + upstream URL),
// so its files are classified `origin='external'` at index time — the project's
// own resolution rate must not be measured against vendored library internals.
//
// Manifest-driven: the submodule `path =` entries ARE the declaration. No
// path-segment guessing, no per-library name lists. Unlike the on-disk-walking
// `Ecosystem` impls, these files already live in the project tree and are walked
// by the main scan; this module only supplies the external-classification signal
// consumed during origin assignment in `indexer/full.rs`.
// =============================================================================

use std::path::Path;

#[cfg(test)]
#[path = "vendored_submodules_tests.rs"]
mod tests;

/// Submodule subtree paths declared in `<project_root>/.gitmodules`, normalized
/// to forward slashes with surrounding slashes trimmed. Empty when the file is
/// absent or declares no `path` entries.
pub fn submodule_paths(project_root: &Path) -> Vec<String> {
    match std::fs::read_to_string(project_root.join(".gitmodules")) {
        Ok(text) => parse_submodule_paths(&text),
        Err(_) => Vec::new(),
    }
}

/// Parse the `path = <dir>` entries out of `.gitmodules` text. `.gitmodules` is
/// git-config syntax; each `[submodule "..."]` stanza's checkout directory is
/// its `path` key — `url`/`branch`/etc. are ignored.
pub(crate) fn parse_submodule_paths(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "path" {
            continue;
        }
        let p = val.trim().replace('\\', "/");
        let p = p.trim_matches('/');
        if !p.is_empty() {
            out.push(p.to_string());
        }
    }
    out
}

/// True when `rel_path` (project-root-relative) falls inside one of the declared
/// submodule subtrees. Matches the subtree exactly or any descendant — a prefix
/// sibling (`base` vs `baseline/x`) does NOT match.
pub fn is_under_submodule(rel_path: &str, submodule_paths: &[String]) -> bool {
    if submodule_paths.is_empty() {
        return false;
    }
    let norm = rel_path.replace('\\', "/");
    submodule_paths.iter().any(|p| {
        norm == *p
            || norm
                .strip_prefix(p.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    })
}
