// =============================================================================
// engine/rules/self_keyword — profile-declared receiver refs
//
// A bare profile-declared receiver resolves to either the enclosing type or its
// direct parent. Concrete spellings and their semantics are language data.
//
// `enclosing_type` walks the scope chain innermost-first then falls back to
// the source symbol's `scope_path`; it accepts whichever scope is a type kind.
// =============================================================================

use crate::indexer::resolve::engine::contract::Symbol;
use crate::indexer::resolve::engine::kinds::is_type_kind;
use crate::indexer::resolve::engine::{BinderContext, LookupResult, LookupRule};
use crate::type_checker::profile::language_profile::ReceiverRole;

pub struct SelfKeywordRule;

/// The nearest enclosing type symbol for the ref's source symbol. Mirrors
/// `DefaultResolver::enclosing_type` exactly: structured containment edge first,
/// then scope_chain innermost-first, then `scope_path`.
fn enclosing_type<'a>(ctx: &'a BinderContext<'_>) -> Option<&'a Symbol> {
    let lk = ctx.lookup;
    // Structured first: the containment edge names the enclosing type by kind,
    // derived from the `parent_index` chain — immune to qname-assembly bugs.
    if let Some(type_qname) = lk.enclosing_type_qname(&ctx.ref_ctx.source_symbol.qualified_name) {
        if let Some(sym) = lk.by_qualified_name(type_qname) {
            if is_type_kind(&sym.kind) {
                return Some(sym);
            }
        }
    }
    // Fallback for lookups with no containment chain (synthetic test doubles):
    // scope_chain innermost-first, then the source's scope_path.
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

impl LookupRule for SelfKeywordRule {
    fn name(&self) -> &'static str {
        "self_keyword"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let Some(role) = ctx.profile.receiver_role(ctx.target()) else {
            return LookupResult::Pass;
        };
        let edge_kind = ctx.edge_kind();
        let Some(enclosing) = enclosing_type(ctx) else {
            return LookupResult::Pass;
        };
        match role {
            ReceiverRole::EnclosingType => {
                if (ctx.kind)(edge_kind, &enclosing.kind) {
                    LookupResult::Resolved(ctx.resolved(enclosing.id, "engine_self_keyword"))
                } else {
                    LookupResult::Pass
                }
            }
            ReceiverRole::DirectParent => {
                // The direct parent by id, not a qname re-search: a base whose
                // qname is shared with an unrelated type in another package binds
                // the SPECIFIC parent recorded for this child.
                let Some(parent) = ctx
                    .lookup
                    .parent_class_id(enclosing.id)
                    .and_then(|pid| ctx.lookup.symbol_by_id(pid))
                else {
                    return LookupResult::Pass;
                };
                if (ctx.kind)(edge_kind, &parent.kind) {
                    LookupResult::Resolved(ctx.resolved(parent.id, "engine_self_keyword"))
                } else {
                    LookupResult::Pass
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "self_keyword_tests.rs"]
mod tests;
