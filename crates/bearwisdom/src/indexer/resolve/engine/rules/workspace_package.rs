// =============================================================================
// engine/rules/workspace_package — sibling workspace-package scoped bind
//
// When a ref's module (or the import that binds its name) is a BARE module
// specifier (`@org/utils`, not `./utils`) that resolves to a sibling workspace
// package, the target is scoped to that package's symbol set.  Deep imports
// (`@org/utils/sub/mod`) are supported: `workspace_sub_path` peels trailing
// segments to find the package root, then the sub-path filters to symbols whose
// file contains it.
//
// Gated on `profile.workspace_packages`.  Inlined helpers:
//   `workspace_sub_path` — peels a deep import to the sub-path remainder.
//   `is_bare_module_specifier` — rejects relative and drive-rooted specifiers.
// =============================================================================

use crate::indexer::resolve::engine::support::{follow_reexports, workspace_pkg_barrels};
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};

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
        let Some(pkg_id) = ctx.lookup.workspace_package_id(specifier) else {
            return LookupResult::Pass;
        };
        let sub_path = workspace_sub_path(specifier, ctx.lookup);

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

/// The sub-path remainder after the longest declared workspace-package prefix
/// that `specifier` starts with.  `None` when `specifier` IS a declared name
/// (no sub-path) or when no workspace package matches.
fn workspace_sub_path(specifier: &str, lookup: &dyn crate::indexer::resolve::engine::contract::SymbolLookup) -> Option<String> {
    if lookup.is_workspace_declared_name(specifier) {
        return None;
    }
    let mut path = specifier;
    while let Some(slash) = path.rfind('/') {
        path = &path[..slash];
        if lookup.is_workspace_declared_name(path) {
            return Some(specifier[path.len() + 1..].to_string());
        }
    }
    None
}

#[cfg(test)]
#[path = "workspace_package_tests.rs"]
mod tests;
