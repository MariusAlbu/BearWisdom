//! npm-owned resolver manifest contribution.

use std::collections::HashMap;

use crate::ecosystem::manifest::resolver_policy::ResolverManifestPolicy;
use crate::ecosystem::manifest::{ManifestData, ManifestKind};

pub(crate) fn contribute(
    manifests: &HashMap<ManifestKind, ManifestData>,
    policy: &mut ResolverManifestPolicy,
) {
    if let Some(manifest) = manifests.get(&ManifestKind::Npm) {
        policy.add_module_rewrites(manifest.path_aliases.clone());
        policy.add_exact_module_rewrites(manifest.exact_path_aliases.clone());
    }
}
