//! Cargo-owned resolver manifest contribution.

use std::collections::HashMap;

use crate::ecosystem::manifest::resolver_policy::ResolverManifestPolicy;
use crate::ecosystem::manifest::{ManifestData, ManifestKind};

pub(crate) fn contribute(
    manifests: &HashMap<ManifestKind, ManifestData>,
    policy: &mut ResolverManifestPolicy,
) {
    if let Some(manifest) = manifests.get(&ManifestKind::Cargo) {
        policy.add_package_aliases(manifest.dep_renames.clone());
    }
}
