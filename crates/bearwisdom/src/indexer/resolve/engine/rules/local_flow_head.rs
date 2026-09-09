// =============================================================================
// engine/rules/local_flow_head — chain-less ref resolves through its own
// flow-seeded callable-head pointer
//
// A chain-less ref (no member access after it — a bare call, a bare
// construction) whose target name was seeded into the per-file
// `local_callable_head` cache resolves to the declaration that name points at,
// when it is compatible with the ref's edge kind.
//
// Covers a local binding whose value IS a specific declaration but that
// declaration carries no field/return type of its own to walk further —
// `const { info } = makeLogger()` records `info` → `makeLogger$Ret.info` (see
// `chain::callable_member_qname_on`); a later bare call `info("hi")` binds to
// that exact member instead of falling through to an unrelated same-named
// sibling.
//
// Runs before `SameFileRule` — a name the flow cache actually typed is
// strictly more specific evidence than "any same-named symbol in the file".
// =============================================================================

use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};

pub struct LocalFlowHeadRule;

impl LookupRule for LocalFlowHeadRule {
    fn name(&self) -> &'static str {
        "local_flow_head"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        if target.is_empty() {
            return LookupResult::Pass;
        }
        let symbol = ctx
            .lookup
            .local_callable_id(target)
            .and_then(|id| ctx.lookup.symbol_by_id(id))
            .or_else(|| {
                ctx.lookup
                    .local_callable_head(target)
                    .and_then(|qname| ctx.lookup.by_qualified_name(&qname))
            });
        let Some(sym) = symbol else {
            return LookupResult::Pass;
        };
        if (ctx.kind)(ctx.edge_kind(), &sym.kind) {
            LookupResult::Resolved(ctx.resolved(sym.id, "local_flow_head"))
        } else {
            LookupResult::Pass
        }
    }
}

#[cfg(test)]
#[path = "local_flow_head_tests.rs"]
mod tests;
