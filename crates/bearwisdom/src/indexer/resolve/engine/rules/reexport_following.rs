// =============================================================================
// engine/rules/reexport_following — follow project-internal re-export chains
//
// When a file imports a name (or wildcard-imports a module) and that module
// does not define the name but re-exports it (`export { X } from './y'`,
// `pub use crate::bar::X`, `export * from './z'`), walk the re-export chain
// to the module that actually defines the name.
//
// Only follows INTERNAL re-export hops; entries tagged `is_reexport=false` are
// never in the map. Cross-package re-exports are `reexport_chain`'s job.
// =============================================================================

use crate::indexer::resolve::engine::support::{
    follow_reexports, is_relative_specifier, relative_reexport_candidates,
};
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};

pub struct ReexportFollowingRule;

impl LookupRule for ReexportFollowingRule {
    fn name(&self) -> &'static str {
        "reexport_following"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        if target.is_empty() {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();
        let from_file = ctx.file_ctx.file_path.as_str();

        for import in &ctx.file_ctx.imports {
            if !import.is_wildcard
                && import.imported_name != "*"
                && import.imported_name != target
            {
                continue;
            }
            let Some(module) = import.module_path.as_deref() else {
                continue;
            };
            if module.is_empty() {
                continue;
            }
            // The per-source module map resolves the import to a barrel file
            // directly when present. When it does not (the relative specifier
            // carries no mapping), follow the specifier as-is — an index keyed on
            // the literal spec surfaces here — then join the relative specifier
            // against the source file's directory and follow each indexed
            // re-exporting barrel it could name.
            if let Some(resolved) = ctx.lookup.resolve_module_from(from_file, module) {
                if let Some(res) = follow_reexports(
                    &resolved.to_string(),
                    target,
                    edge_kind,
                    ctx.kind,
                    ctx.lookup,
                    0,
                ) {
                    return LookupResult::Resolved(res);
                }
            } else if is_relative_specifier(module) {
                if let Some(res) =
                    follow_reexports(module, target, edge_kind, ctx.kind, ctx.lookup, 0)
                {
                    return LookupResult::Resolved(res);
                }
                for barrel in relative_reexport_candidates(ctx.lookup, from_file, module) {
                    if let Some(res) =
                        follow_reexports(&barrel, target, edge_kind, ctx.kind, ctx.lookup, 0)
                    {
                        return LookupResult::Resolved(res);
                    }
                }
            }
        }
        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "reexport_following_tests.rs"]
mod tests;
