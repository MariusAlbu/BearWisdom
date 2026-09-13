// =============================================================================
// indexer/external_root_demand.rs — widen each dep root's demand to its module's
// workspace-wide demand
// =============================================================================

use std::collections::{BTreeMap, BTreeSet};

use crate::ecosystem::externals::ExternalDepRoot;

/// Whether `spec` is a specifier this module itself would satisfy: the bare
/// module specifier, or a subpath beneath it. Demand phrased in another
/// namespace (dotted fully-qualified names, class names) is left with the root
/// that collected it, so unioning is a no-op for ecosystems whose demand is
/// not module-prefixed.
///
/// The prefix match is segment-anchored on `/`: module `pkg` never claims
/// `pkg-extra/x`.
fn is_own_module_demand(module_path: &str, spec: &str) -> bool {
    spec == module_path
        || spec
            .strip_prefix(module_path)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Give every root the union of the own-module demand collected by all roots
/// naming the same `(ecosystem, module_path)`, so a copy of a module found by
/// one workspace package is probed for the subpaths another package asked for.
///
/// A dep root is a physical directory shared across the workspace while
/// `requested_imports` is a per-discovering-package scan result; without this
/// widening the first package to reach a directory decides what the whole
/// workspace may resolve through it.
///
/// Output order is sorted, preserving the determinism contract that
/// `requested_imports` order drives first-writer-wins selection downstream.
pub(crate) fn union_module_demand(roots: &mut [ExternalDepRoot]) {
    let mut by_module: BTreeMap<(&'static str, String), BTreeSet<String>> = BTreeMap::new();
    for root in roots.iter() {
        let shared = by_module
            .entry((root.ecosystem, root.module_path.clone()))
            .or_default();
        for spec in &root.requested_imports {
            if is_own_module_demand(&root.module_path, spec) {
                shared.insert(spec.clone());
            }
        }
    }
    for root in roots.iter_mut() {
        let Some(shared) = by_module.get(&(root.ecosystem, root.module_path.clone())) else {
            continue;
        };
        let mut merged: BTreeSet<String> = root.requested_imports.iter().cloned().collect();
        merged.extend(shared.iter().cloned());
        root.requested_imports = merged.into_iter().collect();
    }
}

#[cfg(test)]
#[path = "external_root_demand_tests.rs"]
mod tests;
