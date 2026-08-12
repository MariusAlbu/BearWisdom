// =============================================================================
// engine/rules/wildcard_workspace_package — bare name under a glob-imported
// workspace package
//
// A `*` wildcard import whose module specifier names a workspace package —
// a sibling by declared name, or this file's own via `profile.imports.self_package_root`
// — brings that package's public symbols into bare-name scope. Reached lazily,
// per failing bare name, rather than eagerly enumerating every glob's full
// symbol set up front.
//
// Scoped the same way `WorkspacePackageRule` scopes an EXPLICIT import:
// package id + optional sub-path file-substring match (`workspace_sub_path`),
// never a `qualified_name` string test. That distinction is the reason this
// rule exists apart from `WildcardImportRule`'s `QnameUnder` mode: a name
// reached through a `pub use` re-export from a deeper submodule shares no
// qname prefix with the glob's module path, but its FILE still sits under
// that module's directory regardless of how many re-export hops away it's
// declared.
//
// Gated on `profile.imports.wildcard_workspace_scope`. Accepts only when EXACTLY ONE
// candidate matches across ALL of the file's wildcard imports — ambiguity
// (two same-named candidates, whether under one glob or split across two)
// stays unresolved rather than guessing.
// =============================================================================

use crate::indexer::resolve::engine::support::{
    is_bare_module_specifier, self_package_sub_path, workspace_sub_path,
};
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};

pub struct WildcardWorkspacePackageRule;

impl LookupRule for WildcardWorkspacePackageRule {
    fn name(&self) -> &'static str {
        "wildcard_workspace_package"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        if !ctx.profile.imports.wildcard_workspace_scope {
            return LookupResult::Pass;
        }
        let target = ctx.target();
        if target.is_empty() {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();

        let mut hits: Vec<i64> = Vec::new();
        for imp in ctx.file_ctx.imports.iter().filter(|imp| imp.is_wildcard) {
            let Some(specifier) = imp.module_path.as_deref().filter(|m| !m.is_empty()) else {
                continue;
            };
            if !is_bare_module_specifier(specifier) {
                continue;
            }
            let (pkg_id, sub_path) = match self_package_sub_path(ctx.profile, specifier) {
                Some(sub_path) => (ctx.ref_ctx.file_package_id, sub_path),
                None => (
                    ctx.lookup.workspace_package_id(specifier),
                    workspace_sub_path(specifier, ctx.lookup),
                ),
            };
            let Some(pkg_id) = pkg_id else {
                continue;
            };
            for sym in ctx.lookup.symbols_in_package(pkg_id) {
                if sym.name != target || !(ctx.kind)(edge_kind, &sym.kind) {
                    continue;
                }
                let under_glob = match sub_path.as_deref() {
                    Some(sub) => sym.file_path.contains(sub),
                    None => true,
                };
                if under_glob {
                    hits.push(sym.id);
                }
            }
        }
        hits.sort_unstable();
        hits.dedup();
        if hits.len() == 1 {
            return LookupResult::Resolved(ctx.resolved(hits[0], "wildcard_workspace_package"));
        }
        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "wildcard_workspace_package_tests.rs"]
mod tests;
