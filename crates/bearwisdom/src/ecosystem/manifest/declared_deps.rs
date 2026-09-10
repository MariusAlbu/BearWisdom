//! Dependency-declaration evidence, routed through ecosystem-owned spelling.

use rustc_hash::{FxHashMap, FxHashSet};

use super::ManifestKind;
use crate::indexer::project_context::ProjectContext;

#[derive(Debug)]
struct ManifestDependencies {
    kind: ManifestKind,
    names: FxHashSet<String>,
}

/// Dependency names scoped to the package whose manifest declares them.
/// Names stay in their manifest form; ecosystem adapters interpret source
/// spelling only for languages they own.
#[derive(Debug, Default)]
pub(crate) struct DeclaredDeps {
    by_package: FxHashMap<i64, Vec<ManifestDependencies>>,
    union: Vec<ManifestDependencies>,
}

impl DeclaredDeps {
    pub(crate) fn snapshot(ctx: &ProjectContext) -> Self {
        let mut out = Self::default();
        for (&pkg_id, manifests) in &ctx.by_package {
            out.by_package.insert(pkg_id, snapshot_manifests(manifests));
        }
        out.union = snapshot_manifests(&ctx.manifests);
        out
    }

    /// Checks declared-dependency evidence using the source language's owning
    /// package ecosystem. A language/ecosystem pair without an adapter gets a
    /// deliberately exact fallback; it cannot inherit npm deep-path or PyPI
    /// normalization rules.
    pub(crate) fn contains(&self, package_id: Option<i64>, language: &str, spec: &str) -> bool {
        let manifests = package_id
            .and_then(|id| self.by_package.get(&id))
            .unwrap_or(&self.union);
        manifests
            .iter()
            .any(|manifest| dependency_matches(manifest.kind, &manifest.names, language, spec))
    }
}

fn snapshot_manifests(
    manifests: &std::collections::HashMap<ManifestKind, super::ManifestData>,
) -> Vec<ManifestDependencies> {
    manifests
        .iter()
        .map(|(&kind, manifest)| ManifestDependencies {
            kind,
            names: manifest.dependencies.iter().cloned().collect(),
        })
        .collect()
}

fn dependency_matches(
    kind: ManifestKind,
    names: &FxHashSet<String>,
    language: &str,
    spec: &str,
) -> bool {
    crate::ecosystem::npm::declared_deps::matches(kind, names, language, spec)
        || crate::ecosystem::cargo::declared_deps::matches(kind, names, language, spec)
        || crate::ecosystem::nuget::declared_deps::matches(kind, names, language, spec)
        || crate::ecosystem::pypi::declared_deps::matches(kind, names, language, spec)
        || names.contains(spec)
}

#[cfg(test)]
#[path = "declared_deps_tests.rs"]
mod tests;
