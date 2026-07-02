// =============================================================================
// engine/rules/workspace_package — sibling workspace-package scoped bind
//
// When a ref's module (or the import that binds its name) is a BARE module
// specifier (`@org/utils`, not `./utils`) that resolves to a sibling workspace
// package, the target is scoped to that package's symbol set.  Deep imports
// (`@org/utils/sub/mod`) are supported: `support::workspace_sub_path` peels
// trailing segments to find the package root, then the sub-path filters to
// symbols whose file contains it.
//
// A specifier led by `profile.self_package_root` (Rust's `crate`) names the
// CURRENT file's own package rather than a sibling by declared name —
// `self_package_sub_path` resolves it against `file_package_id` directly
// instead of `workspace_package_id`'s declared-name table, so a name
// re-exported at the package root binds the same way a direct declaration
// would (both are members of the same package's symbol set).
//
// Gated on `profile.workspace_packages`.  `is_bare_module_specifier` (inlined
// below) rejects relative and drive-rooted specifiers.
// =============================================================================

use crate::indexer::resolve::engine::support::{
    follow_reexports, workspace_pkg_barrels, workspace_sub_path,
};
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::LanguageProfile;

pub struct WorkspacePackageRule;

impl LookupRule for WorkspacePackageRule {
    fn name(&self) -> &'static str {
        "workspace_package"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        if !ctx.profile.workspace_packages {
            return LookupResult::Pass;
        }
        let target = ctx.target();
        if target.is_empty() {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();

        // The specifier comes from the ref's own module field first, then from
        // the import that binds this target by name.
        let specifier: Option<&str> = match ctx.r().module.as_deref() {
            Some(m) => Some(m),
            None => ctx
                .file_ctx
                .imports
                .iter()
                .find(|imp| imp.imported_name == target)
                .and_then(|imp| imp.module_path.as_deref()),
        };
        let Some(specifier) = specifier else {
            return LookupResult::Pass;
        };
        if !is_bare_module_specifier(specifier) {
            return LookupResult::Pass;
        }
        let (pkg_id, sub_path) = match self_package_sub_path(ctx.profile, specifier) {
            Some(sub_path) => (ctx.ref_ctx.file_package_id, sub_path),
            None => (
                ctx.lookup.workspace_package_id(specifier),
                workspace_sub_path(specifier, ctx.lookup),
            ),
        };
        let Some(pkg_id) = pkg_id else {
            return LookupResult::Pass;
        };

        let mut fallback: Option<i64> = None;
        for sym in ctx.lookup.symbols_in_package(pkg_id) {
            if sym.name != target || !(ctx.kind)(edge_kind, &sym.kind) {
                continue;
            }
            if let Some(sub) = sub_path.as_deref() {
                if sym.file_path.contains(sub) {
                    return LookupResult::Resolved(
                        ctx.resolved(sym.id, "default_workspace_package"),
                    );
                }
            }
            if fallback.is_none() {
                fallback = Some(sym.id);
            }
        }
        if let Some(id) = fallback {
            return LookupResult::Resolved(ctx.resolved(id, "default_workspace_package"));
        }

        // The package re-exports the name through its public barrel but does not
        // declare it — `export * from '@org/core'` forwards a sibling workspace
        // package's symbol. Follow the re-export chain from each of the package's
        // `index` barrels to the declaring symbol, which may live in another
        // workspace package. (The bare specifier has no `resolve_module_from`
        // mapping, so the barrel is recovered from the package's own symbol set.)
        for barrel in workspace_pkg_barrels(ctx.lookup, specifier) {
            if let Some(res) =
                follow_reexports(&barrel, target, edge_kind, ctx.kind, ctx.lookup, 0)
            {
                return LookupResult::Resolved(res);
            }
        }
        LookupResult::Pass
    }
}

// =============================================================================
// Private helpers
// =============================================================================

/// A bare module specifier names a package, not a project-relative path.
/// Rejects specifiers that start with `.`, `/`, or a Windows drive letter
/// (`C:/`).
fn is_bare_module_specifier(spec: &str) -> bool {
    !spec.starts_with('.')
        && !spec.starts_with('/')
        && !(spec.len() >= 2 && spec.as_bytes()[1] == b':')
}

/// When `specifier`'s leading segment is `profile.self_package_root`
/// (Rust's `crate`), the sub-path remainder that follows it: `Some(None)` for
/// the bare keyword (`crate`), `Some(Some(rest))` for a deeper path
/// (`crate::thing` -> `Some(Some("thing"))`). `None` when the profile carries
/// no such keyword or `specifier` doesn't lead with it, so the caller falls
/// back to the declared-name lookup.
fn self_package_sub_path(profile: &LanguageProfile, specifier: &str) -> Option<Option<String>> {
    let keyword = profile.self_package_root?;
    let rest = specifier.strip_prefix(keyword)?;
    if rest.is_empty() {
        return Some(None);
    }
    rest.strip_prefix(profile.qname_separator)
        .or_else(|| rest.strip_prefix('/'))
        .map(|sub| Some(sub.to_string()))
}

#[cfg(test)]
#[path = "workspace_package_tests.rs"]
mod tests;
