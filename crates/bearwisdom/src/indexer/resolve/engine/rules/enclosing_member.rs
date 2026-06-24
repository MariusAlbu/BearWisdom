// =============================================================================
// engine/rules/enclosing_member — member declared on an ancestor of the
// enclosing type
//
// `scope_visible` already resolves members of the immediate enclosing scope via
// `{scope}.{target}`. This rule climbs the inheritance chain with
// `parent_class_qname` and accepts a member whose simple name matches — reaching
// inherited fields and methods declared on a base class. The climb is bounded at
// `MAX_INHERITANCE_DEPTH` to prevent infinite loops in cycles.
//
// A dotted or qualified target is declined: only bare names are inherited-member
// candidates.
// =============================================================================

use crate::indexer::resolve::engine::support::is_type_kind;
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::indexer::resolve::engine::contract::Symbol;

/// Maximum inheritance chain depth to walk before giving up. Prevents
/// unbounded iteration on pathological or cyclic inheritance graphs.
const MAX_INHERITANCE_DEPTH: usize = 8;

pub struct EnclosingMemberRule;

/// The nearest enclosing type symbol for the ref's source symbol. Identical to
/// the same helper in `self_keyword` — inlined so this file is self-contained.
fn enclosing_type<'a>(ctx: &'a BinderContext<'_>) -> Option<&'a Symbol> {
    let lk = ctx.lookup;
    if let Some(type_qname) =
        lk.enclosing_type_qname(&ctx.ref_ctx.source_symbol.qualified_name)
    {
        if let Some(sym) = lk.by_qualified_name(type_qname) {
            if is_type_kind(&sym.kind) {
                return Some(sym);
            }
        }
    }
    for scope in &ctx.ref_ctx.scope_chain {
        if let Some(sym) = lk.by_qualified_name(scope) {
            if is_type_kind(&sym.kind) {
                return Some(sym);
            }
        }
    }
    let sp = ctx.ref_ctx.source_symbol.scope_path.as_deref()?;
    let sym = lk.by_qualified_name(sp)?;
    is_type_kind(&sym.kind).then_some(sym)
}

impl LookupRule for EnclosingMemberRule {
    fn name(&self) -> &'static str {
        "enclosing_member"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        if target.is_empty() || target.contains('.') || target.contains("::") {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();
        let Some(enc) = enclosing_type(ctx) else {
            return LookupResult::Pass;
        };
        // Climb the inheritance DAG by symbol id, not by qname string: a base
        // whose qname is shared with an unrelated type in another package would
        // otherwise resolve the wrong type's members via a first-winner re-search.
        // BFS over `parent_class_ids` reaches a member declared on ANY supertype.
        let mut seen: Vec<i64> = Vec::new();
        let mut frontier: Vec<i64> = vec![enc.id];
        for _ in 0..MAX_INHERITANCE_DEPTH {
            if frontier.is_empty() {
                break;
            }
            let mut next: Vec<i64> = Vec::new();
            for id in frontier.drain(..) {
                if seen.contains(&id) {
                    continue;
                }
                seen.push(id);
                for member in ctx.lookup.members_of_id(id) {
                    if member.name == target && (ctx.kind)(edge_kind, &member.kind) {
                        return LookupResult::Resolved(
                            ctx.resolved(member.id, "engine_enclosing_member"),
                        );
                    }
                }
                next.extend(ctx.lookup.parent_class_ids(id));
            }
            frontier = next;
        }
        LookupResult::Pass
    }
}

#[cfg(test)]
#[path = "enclosing_member_tests.rs"]
mod tests;
