//! Workspace package indexes derived once from the scanned packages: declared
//! name → package id, the canonical spelling back from an id, package id →
//! root path, the project-relative entry candidates its manifest declares, and
//! the directory its module sub-paths are rooted at.
use std::collections::HashMap;

use tracing::warn;

use crate::ecosystem::manifest::{ManifestData, ManifestKind};
use crate::types::PackageInfo;

#[derive(Debug, Clone, Default)]
pub struct WorkspaceIndexes {
    pub by_declared_name: HashMap<String, i64>,
    /// The one spelling a package declared for itself — the inverse of the
    /// name a package OWNS in `by_declared_name`, never an ecosystem alias, so
    /// a consumer that hands the name back to an ecosystem gets the spelling
    /// that ecosystem's own manifest used.
    pub declared_name: HashMap<i64, String>,
    pub paths: HashMap<i64, String>,
    pub entries: HashMap<i64, Vec<String>>,
    /// Project-relative directory a package's module sub-paths are rooted at,
    /// for the packages whose manifest declares one.
    pub source_roots: HashMap<i64, String>,
}

/// Build the workspace indexes. Two packages must never share a declared
/// name within one ecosystem — imports key on it — so the first keeps the
/// slot and the duplicate is logged. Ecosystem-supplied name aliases claim a
/// slot only after every package's exact spelling has.
pub(super) fn indexes(
    packages: &[PackageInfo],
    by_package: &HashMap<i64, HashMap<ManifestKind, ManifestData>>,
) -> WorkspaceIndexes {
    let mut out = WorkspaceIndexes::default();
    for pkg in packages {
        let Some(id) = pkg.id else { continue };
        out.paths.insert(id, pkg.path.clone());
        let manifests = by_package.get(&id);
        let candidates = entry_candidates(&pkg.path, manifests);
        if !candidates.is_empty() {
            out.entries.insert(id, candidates);
        }
        if let Some(root) = source_root(&pkg.path, manifests) {
            out.source_roots.insert(id, root);
        }
        let Some(declared) = &pkg.declared_name else {
            continue;
        };
        if declared.is_empty() {
            continue;
        }
        if let Some(existing) = out.by_declared_name.get(declared) {
            warn!(
                "Duplicate declared_name {:?} for packages id={} (kept) and id={} (path={:?}); \
                 imports keyed on this name will route to the first.",
                declared, existing, id, pkg.path,
            );
            continue;
        }
        out.by_declared_name.insert(declared.clone(), id);
        out.declared_name.insert(id, declared.clone());
    }
    let ecosystems = crate::ecosystem::default_registry();
    for pkg in packages {
        let Some(id) = pkg.id else { continue };
        let Some(declared) = &pkg.declared_name else {
            continue;
        };
        let Some(kind) = pkg.kind.as_deref() else {
            continue;
        };
        for alias in ecosystems.workspace_package_name_aliases(kind, declared) {
            out.by_declared_name.entry(alias).or_insert(id);
        }
    }
    out
}

/// The package's declared entry candidates joined onto its root, as
/// project-relative paths in declared priority.
fn entry_candidates(
    package_path: &str,
    manifests: Option<&HashMap<ManifestKind, ManifestData>>,
) -> Vec<String> {
    let root = normalized_root(package_path);
    manifests
        .into_iter()
        .flat_map(|by_kind| by_kind.values())
        .flat_map(|data| data.package_entries.iter())
        .map(|entry| join_under_root(&root, entry))
        .collect()
}

/// The package's declared source root joined onto its own root, as a
/// project-relative directory. `None` when no manifest declares one.
fn source_root(
    package_path: &str,
    manifests: Option<&HashMap<ManifestKind, ManifestData>>,
) -> Option<String> {
    let root = normalized_root(package_path);
    manifests?
        .values()
        .find_map(|data| data.package_source_root.as_deref())
        .map(|declared| join_under_root(&root, declared))
}

/// A package root as a slash-separated path with no leading or trailing slash.
fn normalized_root(package_path: &str) -> String {
    package_path
        .replace('\\', "/")
        .trim_matches('/')
        .to_string()
}

fn join_under_root(root: &str, tail: &str) -> String {
    if root.is_empty() {
        tail.to_string()
    } else {
        format!("{root}/{tail}")
    }
}

#[cfg(test)]
#[path = "project_workspace_tests.rs"]
mod tests;
