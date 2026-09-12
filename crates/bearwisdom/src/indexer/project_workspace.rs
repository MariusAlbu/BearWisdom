//! Workspace package indexes derived once from the scanned packages: declared
//! name → package id, package id → root path, and package id → the
//! project-relative entry candidates its manifest declares.
use std::collections::HashMap;

use tracing::warn;

use crate::ecosystem::manifest::{ManifestData, ManifestKind};
use crate::types::PackageInfo;

pub(super) struct WorkspaceIndexes {
    pub by_declared_name: HashMap<String, i64>,
    pub paths: HashMap<i64, String>,
    pub entries: HashMap<i64, Vec<String>>,
}

/// Build the workspace indexes. Two packages must never share a declared
/// name within one ecosystem — imports key on it — so the first keeps the
/// slot and the duplicate is logged. Ecosystem-supplied name aliases claim a
/// slot only after every package's exact spelling has.
pub(super) fn indexes(
    packages: &[PackageInfo],
    by_package: &HashMap<i64, HashMap<ManifestKind, ManifestData>>,
) -> WorkspaceIndexes {
    let mut by_declared_name: HashMap<String, i64> = HashMap::new();
    let mut paths: HashMap<i64, String> = HashMap::new();
    let mut entries: HashMap<i64, Vec<String>> = HashMap::new();
    for pkg in packages {
        let Some(id) = pkg.id else { continue };
        paths.insert(id, pkg.path.clone());
        let candidates = entry_candidates(&pkg.path, by_package.get(&id));
        if !candidates.is_empty() {
            entries.insert(id, candidates);
        }
        let Some(declared) = &pkg.declared_name else {
            continue;
        };
        if declared.is_empty() {
            continue;
        }
        if let Some(existing) = by_declared_name.get(declared) {
            warn!(
                "Duplicate declared_name {:?} for packages id={} (kept) and id={} (path={:?}); \
                 imports keyed on this name will route to the first.",
                declared, existing, id, pkg.path,
            );
            continue;
        }
        by_declared_name.insert(declared.clone(), id);
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
            by_declared_name.entry(alias).or_insert(id);
        }
    }
    WorkspaceIndexes {
        by_declared_name,
        paths,
        entries,
    }
}

/// The package's declared entry candidates joined onto its root, as
/// project-relative paths in declared priority.
fn entry_candidates(
    package_path: &str,
    manifests: Option<&HashMap<ManifestKind, ManifestData>>,
) -> Vec<String> {
    let root = package_path.replace('\\', "/");
    let root = root.trim_matches('/');
    manifests
        .into_iter()
        .flat_map(|by_kind| by_kind.values())
        .flat_map(|data| data.package_entries.iter())
        .map(|entry| {
            if root.is_empty() {
                entry.clone()
            } else {
                format!("{root}/{entry}")
            }
        })
        .collect()
}
