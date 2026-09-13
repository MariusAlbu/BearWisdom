// =============================================================================
// engine/rules/root_namespace — bare target falling back to the ROOT namespace
//
// Gated on `ctx.profile.root_namespace_fallback` (empty → Pass). A root
// declaration owns no namespace prefix, so its stored qualified name IS the
// bare target: one exact-qname probe reaches it. The profile's kind SET is what
// keeps the fallback honest — a language whose functions and constants fall
// back to the root while its types do not declines the class candidate here.
//
// Sits after the file's own namespace rung, so a same-namespace declaration
// always wins, and before the first-match / proximity rungs, since an exact
// root-qname hit is stronger evidence than either. Two root declarations of one
// name go to the shared ranked chokepoint, which declines below `RANK_MARGIN`
// rather than guessing.
// =============================================================================

use crate::indexer::resolve::engine::contract::Symbol;
use crate::indexer::resolve::engine::support::pick_ranked_candidate;
use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};

pub struct RootNamespaceRule;

impl LookupRule for RootNamespaceRule {
    fn name(&self) -> &'static str {
        "root_namespace"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let kinds = ctx.profile.root_namespace_fallback;
        if kinds.is_empty() {
            return LookupResult::Pass;
        }
        let target = ctx.target();
        if target.is_empty() || ctx.profile.is_qualified_name(target) {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();
        let mut candidates: Vec<&Symbol> = Vec::new();
        for sym in ctx.lookup.all_by_qualified_name(target) {
            if !kinds.iter().any(|k| k.as_str() == sym.kind) {
                continue;
            }
            if !(ctx.kind)(edge_kind, &sym.kind) {
                continue;
            }
            if candidates.iter().any(|c| c.id == sym.id) {
                continue;
            }
            candidates.push(sym);
        }
        match pick_ranked_candidate(
            ctx.file_ctx,
            ctx.ref_ctx.file_package_id,
            ctx.lookup,
            &candidates,
        ) {
            Some(top) => LookupResult::Resolved(ctx.resolved(top.id, "root_namespace")),
            None => LookupResult::Pass,
        }
    }
}

#[cfg(test)]
#[path = "root_namespace_tests.rs"]
mod tests;
