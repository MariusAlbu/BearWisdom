//! Union manifests while retaining each build configuration's package address.
use super::{ManifestData, ManifestKind, PackageManifest};
use std::collections::HashMap;

pub(super) fn union_manifests(
    per_package: &[PackageManifest],
) -> HashMap<ManifestKind, ManifestData> {
    let mut out: HashMap<ManifestKind, ManifestData> = HashMap::new();
    for pm in per_package {
        let entry = out.entry(pm.kind).or_default();
        entry
            .module_packages
            .extend(pm.data.module_packages.iter().cloned());
        entry
            .dependencies
            .extend(pm.data.dependencies.iter().cloned());
        if pm.data.module_path.is_some() {
            entry.module_path = pm.data.module_path.clone();
        }
        entry
            .global_usings
            .extend(pm.data.global_usings.iter().cloned());
        if pm.data.sdk_type.is_some() {
            entry.sdk_type = pm.data.sdk_type.clone();
        }
        for pr in &pm.data.project_refs {
            if !entry.project_refs.contains(pr) {
                entry.project_refs.push(pr.clone());
            }
        }
        // The package's own declared name — its members' names plus the
        // workspace root's — so a reference to a project-own crate is
        // recognized as internal.
        if !pm.name.is_empty() && !entry.package_names.contains(&pm.name) {
            entry.package_names.push(pm.name.clone());
        }
        for alias in &pm.data.path_aliases {
            if !entry.path_aliases.contains(alias) {
                entry.path_aliases.push(alias.clone());
            }
        }
        for t in &pm.data.tsconfig_types {
            if !entry.tsconfig_types.contains(t) {
                entry.tsconfig_types.push(t.clone());
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "project_manifest_union_tests.rs"]
mod tests;
