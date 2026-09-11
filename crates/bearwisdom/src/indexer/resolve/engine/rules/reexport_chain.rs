// =============================================================================
// engine/rules/reexport_chain — cross-package re-export chain resolution
//
// Resolves a ref whose definition lives in a different package than the one
// being imported. Two shapes:
//
//   (a) Direct: `import { Foo } from 'pkg-a'` and target is `Foo` — the symbol
//       is defined in `pkg-b` but re-exported by `pkg-a`.
//   (b) Dotted: `import { Ns } from 'pkg-a'` and target is `Ns.Inner` — the
//       first segment is the import alias; the last segment is the symbol.
//
// Only fires for non-relative (bare package) import specifiers. Relative imports
// are handled by `reexport_following`.
// =============================================================================

use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};
use crate::types::EdgeKind;

pub struct ReexportChainRule;

impl LookupRule for ReexportChainRule {
    fn name(&self) -> &'static str {
        "reexport_chain"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        let edge_kind = ctx.edge_kind();

        // Shape (a): bare target matches a non-relative import — by its BOUND
        // name (a rename binds only the alias). The module-side lookup keys on
        // the import's own declared name, which is what the module re-exports.
        if let Some(matching) = ctx
            .file_ctx
            .imports
            .iter()
            .find(|imp| imp.bound_name() == target)
        {
            if let Some(module) = matching.module_path.as_deref() {
                if !module.is_empty()
                    && !ctx
                        .profile
                        .source_module_path_policy(module)
                        .is_relative(module)
                {
                    let declared = matching.imported_name.as_str();
                    if let Some(id) = ctx
                        .lookup
                        .resolve_external_reexport(declared, declared, module)
                    {
                        if let Some(sid) =
                            candidate_with_compatible_kind(ctx, declared, id, edge_kind)
                        {
                            return LookupResult::Resolved(
                                ctx.resolved(sid, "default_reexport_chain"),
                            );
                        }
                    }
                }
            }
        }

        // Shape (b): profile-qualified target. First segment may be an import alias;
        // last segment is the actual symbol.
        if let Some((prefix, _)) = ctx.profile.split_source_qualified_name(target) {
            let suffix = ctx.profile.simple_name(target);
            if !prefix.is_empty() && !suffix.is_empty() && suffix != target {
                if let Some(matching) = ctx
                    .file_ctx
                    .imports
                    .iter()
                    .find(|imp| imp.imported_name == prefix)
                {
                    if let Some(module) = matching.module_path.as_deref() {
                        if !module.is_empty()
                            && !ctx
                                .profile
                                .source_module_path_policy(module)
                                .is_relative(module)
                        {
                            if let Some(id) =
                                ctx.lookup.resolve_external_reexport(suffix, prefix, module)
                            {
                                if let Some(sid) =
                                    candidate_with_compatible_kind(ctx, suffix, id, edge_kind)
                                {
                                    return LookupResult::Resolved(
                                        ctx.resolved(sid, "default_reexport_chain"),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
        LookupResult::Pass
    }
}

/// Verify that the by-name candidate with `id` is kind-compatible, returning
/// `Some(id)` when it is and `None` when not found or incompatible.
fn candidate_with_compatible_kind(
    ctx: &BinderContext,
    name: &str,
    id: i64,
    edge_kind: EdgeKind,
) -> Option<i64> {
    let candidate = ctx
        .lookup
        .by_name(name)
        .into_iter()
        .find(|sym| sym.id == id)?;
    if (ctx.kind)(edge_kind, &candidate.kind) {
        Some(id)
    } else {
        None
    }
}

#[cfg(test)]
#[path = "reexport_chain_tests.rs"]
mod tests;
