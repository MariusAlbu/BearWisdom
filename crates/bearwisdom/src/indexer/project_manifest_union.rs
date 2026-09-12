//! Union manifests while retaining each build configuration's package address.
use super::{ManifestData, ManifestKind, PackageManifest};
use std::collections::HashMap;

pub(super) fn union_manifests(
    per_package: &[PackageManifest],
) -> HashMap<ManifestKind, ManifestData> {
    let mut out: HashMap<ManifestKind, ManifestData> = HashMap::new();
    for pm in per_package {
        let entry = out.entry(pm.kind).or_default();
        crate::ecosystem::manifest::fold::absorb(entry, &pm.name, &pm.data);
    }
    out
}

#[cfg(test)]
#[path = "project_manifest_union_tests.rs"]
mod tests;
