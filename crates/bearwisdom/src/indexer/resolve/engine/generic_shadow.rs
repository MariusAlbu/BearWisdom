// =============================================================================
// engine/generic_shadow — generic-parameter shadowing
//
// A generic parameter shadows any same-named concrete type inside the scope
// that declares it. Two enforcement points share this module:
//
//   * `GenericParamShadowRule` — a DRAIN guard in the bare-name ladder: a
//     nominal ref (TypeRef / Instantiates / Inherits) whose target names a
//     parameter in scope at the source symbol is fully explained by that
//     declaration. It must not bind a concrete type candidate, and it is not
//     evidence of a resolution gap, so it leaves the rate denominator the
//     same way a builtin drain does.
//
//   * `mark_unbound_member_params` — the member-yield rewrite: a yield that
//     still carries `Class(p)` after receiver- and argument-driven
//     substitution holds an UNBOUND parameter. Rewriting those leaves to
//     arena `Generic` markers strips the nominal head, so the next hop's
//     head lookup cannot bind a same-named concrete class and the walk's
//     miss stays visible as evidence of the missing receiver arguments.
// =============================================================================

use rustc_hash::FxHashMap;

use crate::indexer::resolve::engine::contract::{Symbol, SymbolLookup};
use crate::type_checker::core::types::{GenericParamData, Type, TypeArena, TypeId};
use crate::types::EdgeKind;

use super::{BinderContext, LookupResult, LookupRule};

pub struct GenericParamShadowRule;

impl LookupRule for GenericParamShadowRule {
    fn name(&self) -> &'static str {
        "generic_param_shadow"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        if !matches!(
            ctx.edge_kind(),
            EdgeKind::TypeRef | EdgeKind::Instantiates | EdgeKind::Inherits
        ) {
            return LookupResult::Pass;
        }
        let target = ctx.target();
        if target.is_empty() || target.contains('.') || target.contains("::") {
            return LookupResult::Pass;
        }
        let source = ctx.ref_ctx.source_symbol;
        // Params interned onto the source symbol at extract time, when the
        // index carries an arena; the qname-keyed slots cover the rest.
        let own = ctx.lookup.type_arena().is_some_and(|arena| {
            source
                .generic_params
                .iter()
                .any(|id| arena.generic_param(*id).name == target)
        });
        if !own && !param_in_scope(ctx.lookup, &source.qualified_name, target) {
            return LookupResult::Pass;
        }
        crate::tracef!(
            "  SHADOW '{}' is a generic parameter in scope at {} -> DRAIN",
            target,
            source.qualified_name,
        );
        LookupResult::Drained
    }
}

/// `true` when `name` is a generic parameter declared on the symbol at
/// `source_qname` or on any enclosing owner — every dotted prefix of the
/// qname is consulted, so a member of a generic type sees the type's
/// parameters however deep the nesting.
pub(crate) fn param_in_scope(lookup: &dyn SymbolLookup, source_qname: &str, name: &str) -> bool {
    if declares_param(lookup, source_qname, name) {
        return true;
    }
    let mut prefix = source_qname;
    while let Some((owner, _)) = prefix.rsplit_once('.') {
        if declares_param(lookup, owner, name) {
            return true;
        }
        prefix = owner;
    }
    false
}

/// `true` when the declaration at `qname` introduces a generic parameter
/// `name` — the qname-keyed slot first, then the id-keyed slot of the symbol
/// the qname resolves to.
fn declares_param(lookup: &dyn SymbolLookup, qname: &str, name: &str) -> bool {
    if lookup
        .generic_params(qname)
        .is_some_and(|ps| ps.iter().any(|p| p == name))
    {
        return true;
    }
    lookup
        .by_qualified_name(qname)
        .and_then(|s| lookup.generic_params_of(s.id))
        .is_some_and(|ps| ps.iter().any(|p| p == name))
}

/// Rewrite the parameter names still nominal in `yielded` — `Class(p)` where
/// `p` is a generic parameter of `member` or of its declaring type — to arena
/// `Generic` markers.
///
/// Runs AFTER receiver-driven substitution and argument-driven fill: anything
/// those bound is already concrete, so a surviving `Class(p)` is an unbound
/// parameter. Left nominal, it would let the next hop's head lookup bind a
/// same-named concrete class; a marker has no nominal head, so the receiver
/// stays unbound and the implicit-root gate preserves the miss. Unchanged
/// when no parameter name survives in the yield.
pub(crate) fn mark_unbound_member_params(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    member: &Symbol,
    yielded: TypeId,
) -> TypeId {
    let names = member_param_names(lookup, member);
    if names.is_empty() || !mentions_param(arena, yielded, &names) {
        return yielded;
    }
    crate::tracef!(
        "  SHADOW yield of '{}' keeps unbound params {:?} -> Generic markers",
        member.qualified_name,
        names,
    );
    let map: FxHashMap<String, TypeId> = names
        .into_iter()
        .map(|n| {
            let param = arena.intern_generic(GenericParamData {
                name: n.clone(),
                owner_symbol_index: 0,
                bound: None,
            });
            (n, arena.intern(Type::Generic { param }))
        })
        .collect();
    arena.rebind_class_params(yielded, &map)
}

/// The generic-parameter names in scope for `member`'s declared type: the
/// member's own parameters plus its declaring type's (the qname minus the
/// final segment). Id-keyed slots are preferred over qname slots so a
/// same-qname collision in another package cannot leak params in.
fn member_param_names(lookup: &dyn SymbolLookup, member: &Symbol) -> Vec<String> {
    let mut names = lookup
        .generic_params_of(member.id)
        .or_else(|| lookup.generic_params(&member.qualified_name))
        .unwrap_or_default();
    if let Some((decl, _)) = member.qualified_name.rsplit_once('.') {
        let decl_params = lookup
            .by_qualified_name(decl)
            .and_then(|s| lookup.generic_params_of(s.id))
            .or_else(|| lookup.generic_params(decl))
            .unwrap_or_default();
        for p in decl_params {
            if !names.contains(&p) {
                names.push(p);
            }
        }
    }
    names
}

/// `true` when the type contains a `Class` leaf naming one of `names`. Cheap
/// pre-scan so the marker map (which interns new arena slots) is built only
/// for yields that actually carry a leftover parameter.
fn mentions_param(arena: &TypeArena, id: TypeId, names: &[String]) -> bool {
    match arena.get(id) {
        Type::Class(n) => names.iter().any(|p| *p == n),
        Type::Apply { base, args } => {
            mentions_param(arena, base, names)
                || args.iter().any(|&a| mentions_param(arena, a, names))
        }
        Type::Optional(inner) | Type::AsyncWrapper(inner) | Type::Iterator(inner) => {
            mentions_param(arena, inner, names)
        }
        Type::Union(arms) | Type::Intersection(arms) => {
            arms.iter().any(|&a| mentions_param(arena, a, names))
        }
        Type::Tuple(elems) => elems.iter().any(|&e| mentions_param(arena, e, names)),
        Type::Function { params, return_ } => {
            params.iter().any(|&p| mentions_param(arena, p, names))
                || mentions_param(arena, return_, names)
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "generic_shadow_tests.rs"]
mod tests;
