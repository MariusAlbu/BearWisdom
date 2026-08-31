// =============================================================================
// engine/declared_deps — manifest-declared dependency names, per package
//
// A snapshot of every dependency name the project's manifests declare, keyed
// by workspace package with a workspace-wide union fallback — mirroring
// `ProjectContext::manifests_for` isolation semantics. Evidence for cause
// attribution only: the probe answers "does the manifest visible to this
// file's package declare this module specifier?", it never participates in
// resolution and produces no bindings.
// =============================================================================

use rustc_hash::{FxHashMap, FxHashSet};

use crate::indexer::project_context::ProjectContext;

/// Dependency names declared across the project's manifests, normalized for
/// probe stability (lowercased, `_` folded to `-` so pip-style import names
/// match their PyPI-canonical manifest spelling).
#[derive(Debug, Default)]
pub(super) struct DeclaredDeps {
    /// Per isolated package: only that package's own manifests.
    by_package: FxHashMap<i64, FxHashSet<String>>,
    /// Workspace-wide union — the fallback for files outside every package.
    union: FxHashSet<String>,
}

impl DeclaredDeps {
    pub(super) fn snapshot(ctx: &ProjectContext) -> Self {
        let mut out = Self::default();
        for (&pkg_id, manifests) in &ctx.by_package {
            let set: FxHashSet<String> = manifests
                .values()
                .flat_map(|m| m.dependencies.iter())
                .map(|d| normalize(d))
                .collect();
            out.by_package.insert(pkg_id, set);
        }
        out.union = ctx
            .manifests
            .values()
            .flat_map(|m| m.dependencies.iter())
            .map(|d| normalize(d))
            .collect();
        out
    }

    /// Whether the manifest visible to `package_id` declares `spec` — probed
    /// as the full specifier, then as its bare package head (`lodash/fp` →
    /// `lodash`, `@scope/pkg/sub` → `@scope/pkg`). A package present in the
    /// per-package map is isolated to its own set; anything else falls back
    /// to the union, mirroring `manifests_for`.
    pub(super) fn contains(&self, package_id: Option<i64>, spec: &str) -> bool {
        let set = package_id
            .and_then(|id| self.by_package.get(&id))
            .unwrap_or(&self.union);
        if set.contains(&normalize(spec)) {
            return true;
        }
        bare_head(spec).is_some_and(|head| set.contains(&normalize(head)))
    }
}

fn normalize(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', "-")
}

/// The leading package segment of a deep bare specifier: two segments for a
/// scoped name, one otherwise. `None` when there is nothing shorter than the
/// full specifier to probe (relative paths, bare one-segment names).
fn bare_head(spec: &str) -> Option<&str> {
    if spec.starts_with('.') {
        return None;
    }
    let mut slashes = spec.match_indices('/').map(|(i, _)| i);
    if spec.starts_with('@') {
        slashes.next()?;
        slashes.next().map(|second| &spec[..second])
    } else {
        slashes.next().map(|first| &spec[..first])
    }
}

#[cfg(test)]
#[path = "declared_deps_tests.rs"]
mod tests;
