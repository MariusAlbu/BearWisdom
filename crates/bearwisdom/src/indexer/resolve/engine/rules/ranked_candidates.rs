// =============================================================================
// engine/rules/ranked_candidates — proximity-scored tiebreaker for bare names
//
// Gated on `ctx.profile.multi_candidate_ranking` (false → Pass). Fires only
// when `by_name` yields two or more kind-compatible candidates for a bare
// (separator-free) target — single-candidate refs are left to the earlier
// strict-name rules. Scores each candidate by workspace-package membership,
// import-module proximity, ambient status, path proximity, external-path depth
// penalty, and visibility, then accepts the top scorer only when it beats the
// runner-up by at least `RANK_MARGIN`. A margin smaller than that leaves the
// ref honestly unresolved rather than guessing.
// =============================================================================

use crate::indexer::resolve::engine::support::pick_ranked_candidate;
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::indexer::resolve::engine::contract::Symbol;

pub struct RankedCandidatesRule;

impl LookupRule for RankedCandidatesRule {
    fn name(&self) -> &'static str {
        "ranked_candidates"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        if !ctx.profile.multi_candidate_ranking {
            return LookupResult::Pass;
        }
        let target = ctx.target();
        if target.is_empty()
            || target.contains('.')
            || target.contains("::")
            || target.contains('/')
        {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();
        let candidates: Vec<&Symbol> = ctx
            .lookup
            .by_name(target)
            .into_iter()
            .filter(|sym| (ctx.kind)(edge_kind, &sym.kind))
            .collect();
        // Zero or one: the earlier strict rules had their chance. Stay out.
        if candidates.len() < 2 {
            return LookupResult::Pass;
        }
        // The shared import-scoped chokepoint: the top scorer past the margin,
        // else honestly unresolved rather than a first-wins guess.
        match pick_ranked_candidate(
            ctx.file_ctx,
            ctx.ref_ctx.file_package_id,
            ctx.lookup,
            &candidates,
        ) {
            Some(top) => LookupResult::Resolved(ctx.resolved(top.id, "default_ranked_candidate")),
            None => LookupResult::Pass,
        }
    }
}

#[cfg(test)]
#[path = "ranked_candidates_tests.rs"]
mod tests;
