// =============================================================================
// ecosystem/manifest/fold.rs — folding one package's manifest data into an
// ecosystem-wide entry
// =============================================================================

use super::ManifestData;

/// Fold `data`, read for the package named `name`, into `entry`.
///
/// Set-valued axes extend; single-valued axes take the last declared value;
/// list axes that name things (packages, aliases, types, refs, renames)
/// append without duplicates so repeated folds stay idempotent.
pub(crate) fn absorb(entry: &mut ManifestData, name: &str, data: &ManifestData) {
    entry
        .module_packages
        .extend(data.module_packages.iter().cloned());
    entry.dependencies.extend(data.dependencies.iter().cloned());
    if data.module_path.is_some() {
        entry.module_path = data.module_path.clone();
    }
    entry
        .global_usings
        .extend(data.global_usings.iter().cloned());
    if data.sdk_type.is_some() {
        entry.sdk_type = data.sdk_type.clone();
    }
    // The package's own declared name — so a `package:<self>/...` URI is
    // recognized as project-local rather than external.
    if !name.is_empty() && !entry.package_names.iter().any(|n| n == name) {
        entry.package_names.push(name.to_owned());
    }
    push_unique(&mut entry.project_refs, &data.project_refs);
    push_unique(&mut entry.path_aliases, &data.path_aliases);
    push_unique(&mut entry.exact_path_aliases, &data.exact_path_aliases);
    push_unique(&mut entry.package_entries, &data.package_entries);
    push_unique(&mut entry.dep_renames, &data.dep_renames);
    push_unique(&mut entry.tsconfig_types, &data.tsconfig_types);
}

fn push_unique<T: Clone + PartialEq>(into: &mut Vec<T>, from: &[T]) {
    for item in from {
        if !into.contains(item) {
            into.push(item.clone());
        }
    }
}
