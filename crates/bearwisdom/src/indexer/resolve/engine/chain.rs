// =============================================================================
// engine/chain — member-access binding (`a.b.c()`)
//
// Binds `a.b.c()` by rooting the first segment to a TypeSymbol, then walking
// member-by-member: find `b` on the root type (climbing supertypes), advance to
// the type `b` yields — substituting the receiver's type arguments through the
// member's declared type — find `c` there, and so on. The last segment's symbol
// is the resolution. The walk threads a `TypeSymbol`, never a type string:
// `Repository<User>.find()` whose return is `T` advances to `User`.
//
// Structural roots only for now (local-variable type, `this`/`self`, a declared
// annotation, a bare type name); alias expansion, union dispatch and
// import-of-value rooting are added measurement-driven as the rate demands them.
// =============================================================================

use crate::indexer::resolve::engine::contract::{
    intern_yield_type, RefContext, Symbol, SymbolInfo, SymbolLookup, RESOLVED_CONFIDENCE,
};
use crate::types::SegmentKind;

use super::alias;
use super::type_symbol::TypeSymbol;

/// Strategy tag for a member-chain bind produced by the new engine.
const STRATEGY: &str = "rule_chain";

/// Upper bound on supertype-chain climbing when locating an inherited member.
const MAX_SUPERTYPE_DEPTH: usize = 8;

/// Resolve a member chain to its final segment's symbol. `None` when the root
/// can't be typed or a hop has no matching member — an honestly-unresolved
/// chain, diagnosable to the segment where the walk stopped.
pub fn bind_member_access(ref_ctx: &RefContext, lookup: &dyn SymbolLookup) -> Option<SymbolInfo> {
    let chain = ref_ctx.extracted_ref.chain.as_ref()?;
    // A single-segment "chain" carries no receiver to root; the bare ladder
    // handles it. Multi-segment only here.
    if chain.segments.len() < 2 {
        return None;
    }

    let mut current = alias::expand(resolve_root(ref_ctx, lookup, &chain.segments[0])?, lookup);
    let last = chain.segments.len() - 1;

    for (i, seg) in chain.segments.iter().enumerate().skip(1) {
        // The member index keys on the bare head type; a receiver typed
        // `Repository<User>` looks up members under `Repository`.
        let member = lookup_member(lookup, &current.qname, &seg.name, &|_kind| true)?;
        if i == last {
            // Yield the final member's type (with the receiver's type arguments
            // substituted) so a `const x = a.b.c()` binding records x's type and
            // a later `x.method()` can root on it — forward inference compounds.
            let resolved_yield_type = yield_through(lookup, &member, seg.is_call, &current)
                .and_then(|ty| intern_yield_type(Some(ty.qname), lookup));
            return Some(SymbolInfo {
                target_symbol_id: member.id,
                confidence: RESOLVED_CONFIDENCE,
                strategy: STRATEGY,
                resolved_yield_type,
                flow_emit: None,
            });
        }
        // Advance to the type this member yields, substituting the receiver's
        // type arguments for the declaring type's generic parameters, then
        // expand it through any type alias before the next member lookup.
        current = alias::expand(yield_through(lookup, &member, seg.is_call, &current)?, lookup);
    }
    None
}

/// Resolve `member` on `type_qname`, climbing its supertypes up to
/// `MAX_SUPERTYPE_DEPTH`. `accept` gates a candidate by kind. The climb is
/// bounded and cheap, so it is recomputed per call rather than memoized.
pub(crate) fn lookup_member(
    lookup: &dyn SymbolLookup,
    type_qname: &str,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
) -> Option<Symbol> {
    let mut cur = type_qname.to_string();
    for _ in 0..MAX_SUPERTYPE_DEPTH {
        for m in lookup.members_of(&cur) {
            if m.name == member && accept(&m.kind) {
                return Some(m.clone());
            }
        }
        match lookup.parent_class_qname(&cur) {
            Some(parent) => cur = parent.to_string(),
            None => break,
        }
    }
    None
}

/// The type `member` yields when used in a chain, as a TypeSymbol: its return
/// type for a call, else its declared field/property type. The raw type comes
/// from the index as a string (the storage format) and is parsed into a
/// TypeSymbol at this boundary. `None` when the index has no type for it.
pub(crate) fn member_yield_type(
    lookup: &dyn SymbolLookup,
    member_qname: &str,
    is_call: bool,
) -> Option<TypeSymbol> {
    let raw = if is_call {
        lookup.return_type_str(member_qname)?
    } else {
        lookup.field_type_str(member_qname)?
    };
    Some(TypeSymbol::parse(&raw))
}

/// The type a member yields when accessed through `receiver`, with the
/// receiver's applied type arguments substituted for the declaring type's
/// generic parameters: `Repository<User>` receiver + `find(): T` → `User`.
fn yield_through(
    lookup: &dyn SymbolLookup,
    member: &Symbol,
    is_call: bool,
    receiver: &TypeSymbol,
) -> Option<TypeSymbol> {
    let yielded = member_yield_type(lookup, &member.qualified_name, is_call)?;
    let params = lookup.generic_params(&receiver.qname).unwrap_or(&[]);
    Some(yielded.substitute(params, &receiver.type_args))
}

/// Type the chain's root segment. Structural cases only for now:
///   - a local variable's forward-inferred type (`local_type` cache),
///   - a declared annotation captured on the segment (head + split type args),
///   - `this`/`self`/`base` → the enclosing type,
///   - a call root (`makeRepo()`) → the callee's return type,
///   - a bare type name used as a static-access / construction root.
fn resolve_root(
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
    seg: &crate::types::ChainSegment,
) -> Option<TypeSymbol> {
    if let Some(ty) = lookup.local_type(&seg.name) {
        return Some(TypeSymbol::parse(&ty));
    }
    if let Some(ty) = &seg.declared_type {
        return Some(with_segment_args(TypeSymbol::parse(ty), &seg.type_args));
    }
    if matches!(seg.kind, SegmentKind::SelfRef) {
        return lookup
            .enclosing_type_qname(&ref_ctx.source_symbol.qualified_name)
            .map(TypeSymbol::plain);
    }
    if seg.is_call {
        if let Some(ty) = callee_return_type(lookup, &seg.name) {
            return Some(ty);
        }
    }
    // Import-of-value / typed-value root: a value (a `declare const`, an
    // imported binding, a typed field) whose declaration carries a type roots
    // the chain on that type — `initTRPC.create()` roots on `initTRPC`'s builder
    // type even though `initTRPC` is not itself a type name.
    if let Some(ty) = value_root_type(lookup, &seg.name) {
        return Some(ty);
    }
    lookup
        .types_by_name(&seg.name)
        .into_iter()
        .next()
        .map(|s| with_segment_args(TypeSymbol::plain(s.qualified_name.clone()), &seg.type_args))
}

/// Type a chain root that is a *value* by the declared type on its declaration:
/// the first value-kind symbol named `name` whose declaration carries a field
/// type. Runs after the flow-cache and annotation roots, so a local's inferred
/// type still wins; this catches imported / ambient values whose type lives on
/// the declaration rather than at the use site.
fn value_root_type(lookup: &dyn SymbolLookup, name: &str) -> Option<TypeSymbol> {
    for cand in lookup.by_name(name) {
        if is_value_kind(&cand.kind) {
            if let Some(ty) = lookup.field_type_str(&cand.qualified_name) {
                return Some(TypeSymbol::parse(&ty));
            }
        }
    }
    None
}

/// `true` when `kind` names a value whose declared type can root a chain.
fn is_value_kind(kind: &str) -> bool {
    matches!(
        kind,
        "variable" | "constant" | "const" | "field" | "property" | "parameter"
    )
}

/// Type a call root by the callee's return type: `makeRepo()` where
/// `makeRepo(): Repository<User>` types the chain head as `Repository<User>`.
fn callee_return_type(lookup: &dyn SymbolLookup, name: &str) -> Option<TypeSymbol> {
    let callee = lookup
        .by_name(name)
        .into_iter()
        .find(|s| is_callable(&s.kind))?;
    let raw = lookup.return_type_str(&callee.qualified_name)?;
    Some(TypeSymbol::parse(&raw))
}

/// Attach a segment's in-source type arguments to a freshly-typed head when the
/// head didn't already carry its own. A declared annotation reaches the chain
/// pre-split — `repo: Repository<User>` as head `Repository` + args `[User]` —
/// so the args must be reattached, else generic substitution sees no arguments.
fn with_segment_args(mut ty: TypeSymbol, type_args: &[String]) -> TypeSymbol {
    if ty.type_args.is_empty() && !type_args.is_empty() {
        ty.type_args = type_args.iter().map(|a| TypeSymbol::parse(a)).collect();
    }
    ty
}

/// `true` when `kind` names something a call can root on — a free function or a
/// method whose return type carries the chain forward.
fn is_callable(kind: &str) -> bool {
    matches!(kind, "function" | "method")
}

#[cfg(test)]
#[path = "chain_tests.rs"]
mod tests;
