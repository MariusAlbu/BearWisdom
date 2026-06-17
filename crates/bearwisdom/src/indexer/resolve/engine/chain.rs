// =============================================================================
// engine/chain — member-access binding (`a.b.c()`)
//
// Binds `a.b.c()` by rooting the first segment to a type, then walking
// member-by-member: find `b` on the root type (climbing supertypes), advance to
// the type `b` yields — substituting the receiver's type arguments through the
// member's declared type — find `c` there, and so on. The last segment's symbol
// is the resolution. The walk threads a canonical `TypeId` from the workspace
// arena, never a type string: `Repository<User>.find()` whose return is `T`
// advances to `User`. Type expressions enter as strings at the index boundary (a
// stored annotation, a forward-inferred local) and are interned ONCE into a
// TypeId at the root; every hop after is entity-based — arena substitution, not
// per-hop string parsing.
//
// Structural roots only for now (local-variable type, `this`/`self`, a declared
// annotation, a bare type name, an imported value's declared type); alias
// expansion is applied at each hop. Union dispatch is added measurement-driven.
// =============================================================================

use rustc_hash::FxHashMap;

use crate::indexer::resolve::engine::contract::{
    RefContext, Symbol, SymbolInfo, SymbolLookup, RESOLVED_CONFIDENCE,
};
use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::types::SegmentKind;

use super::alias;

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
    // The walk is entity-based — it needs the workspace arena to intern roots
    // and substitute generics. The production lookup and the test double both
    // expose one; a lookup without an arena cannot type a chain.
    let arena = lookup.type_arena()?;

    let mut current = alias::expand(
        resolve_root(ref_ctx, lookup, arena, &chain.segments[0])?,
        lookup,
        arena,
    );
    let last = chain.segments.len() - 1;

    for (i, seg) in chain.segments.iter().enumerate().skip(1) {
        // The member index keys on the bare head type; a receiver typed
        // `Repository<User>` looks up members under `Repository`.
        let head = head_qname(arena, current)?;
        let member = lookup_member(lookup, &head, &seg.name, &|_kind| true)?;
        if i == last {
            // The final member's yield type (with the receiver's type arguments
            // substituted) records x's type for `const x = a.b.c()` so a later
            // `x.method()` can root on it — forward inference compounds.
            let resolved_yield_type = yield_through(lookup, arena, &member, seg.is_call, current);
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
        current = alias::expand(
            yield_through(lookup, arena, &member, seg.is_call, current)?,
            lookup,
            arena,
        );
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

/// The qualified-name head of a type: `Repository` for `Repository<User>`.
/// Looks through generic application and the nullable/async/iterator wrappers so
/// member access on `Promise<User>` / `User?` resolves on `User`. `None` for a
/// structural type with no nominal head (function, tuple, union, …).
pub(crate) fn head_qname(arena: &TypeArena, id: TypeId) -> Option<String> {
    match arena.get(id) {
        Type::Class(q) => Some(q),
        Type::Apply { base, .. } => head_qname(arena, base),
        Type::Optional(inner) | Type::AsyncWrapper(inner) | Type::Iterator(inner) => {
            head_qname(arena, inner)
        }
        _ => None,
    }
}

/// The applied type arguments of a type: `[User]` for `Repository<User>`, empty
/// for a non-generic type. Looks through wrappers the same way `head_qname` does
/// so the args travel with the head they belong to.
pub(crate) fn apply_args(arena: &TypeArena, id: TypeId) -> Vec<TypeId> {
    match arena.get(id) {
        Type::Apply { args, .. } => args,
        Type::Optional(inner) | Type::AsyncWrapper(inner) | Type::Iterator(inner) => {
            apply_args(arena, inner)
        }
        _ => Vec::new(),
    }
}

/// The type `member` yields when used in a chain, as a canonical TypeId: its
/// return type for a call, else its declared field/property type. Reads the
/// interned `*_id` first; falls back to interning the string accessor (the
/// storage format) when only a string is recorded. `None` when the index has no
/// type for it.
pub(crate) fn member_yield_type(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    member_qname: &str,
    is_call: bool,
) -> Option<TypeId> {
    if is_call {
        if let Some(id) = lookup.return_type_id(member_qname) {
            return Some(id);
        }
        return lookup
            .return_type_name(member_qname)
            .map(|s| arena.intern_type_str(s));
    }
    if let Some(id) = lookup.field_type_id(member_qname) {
        return Some(id);
    }
    lookup
        .field_type_name(member_qname)
        .map(|s| arena.intern_type_str(s))
}

/// The type a member yields when accessed through `receiver`, with the
/// receiver's applied type arguments substituted for the declaring type's
/// generic parameters: `Repository<User>` receiver + `find(): T` → `User`.
fn yield_through(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    member: &Symbol,
    is_call: bool,
    receiver: TypeId,
) -> Option<TypeId> {
    let yielded = member_yield_type(lookup, arena, &member.qualified_name, is_call)?;
    Some(substitute_through(lookup, arena, yielded, receiver))
}

/// Substitute the receiver's applied type arguments for the declaring type's
/// generic parameters throughout `yielded`. `Repository<User>` receiver with
/// params `[T]` rebinds `Class("T")` → `User`; a non-generic receiver, or a
/// declaring type with no parameters, leaves `yielded` untouched.
fn substitute_through(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    yielded: TypeId,
    receiver: TypeId,
) -> TypeId {
    let Some(head) = head_qname(arena, receiver) else {
        return yielded;
    };
    let args = apply_args(arena, receiver);
    let params = lookup.generic_params(&head).unwrap_or(&[]);
    if params.is_empty() || args.is_empty() {
        return yielded;
    }
    let map: FxHashMap<String, TypeId> =
        params.iter().cloned().zip(args.iter().copied()).collect();
    arena.rebind_class_params(yielded, &map)
}

/// Type the chain's root segment. Structural cases only for now:
///   - a local variable's forward-inferred type (`local_type` cache),
///   - a declared annotation captured on the segment (head + split type args),
///   - `this`/`self`/`base` → the enclosing type,
///   - a call root (`makeRepo()`) → the callee's return type,
///   - an imported / typed value whose declaration carries a type,
///   - a bare type name used as a static-access / construction root.
/// Each case interns its type expression into the arena once, here at the root.
fn resolve_root(
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    seg: &crate::types::ChainSegment,
) -> Option<TypeId> {
    if let Some(ty) = lookup.local_type(&seg.name) {
        return Some(arena.intern_type_str(&ty));
    }
    if let Some(ty) = &seg.declared_type {
        return Some(with_segment_args(arena, arena.intern_type_str(ty), &seg.type_args));
    }
    if matches!(seg.kind, SegmentKind::SelfRef) {
        return lookup
            .enclosing_type_qname(&ref_ctx.source_symbol.qualified_name)
            .map(|q| arena.class(q));
    }
    if seg.is_call {
        if let Some(ty) = callee_return_type(lookup, arena, &seg.name) {
            return Some(ty);
        }
    }
    // Import-of-value / typed-value root: a value (a `declare const`, an
    // imported binding, a typed field) whose declaration carries a type roots
    // the chain on that type — `initTRPC.create()` roots on `initTRPC`'s builder
    // type even though `initTRPC` is not itself a type name.
    if let Some(ty) =
        value_root_type(lookup, arena, &seg.name, &ref_ctx.source_symbol.qualified_name)
    {
        return Some(ty);
    }
    lookup
        .types_by_name(&seg.name)
        .into_iter()
        .next()
        .map(|s| with_segment_args(arena, arena.class(&s.qualified_name), &seg.type_args))
}

/// Type a chain root that is a *value* by the declared type on its declaration.
/// Runs after the flow-cache and annotation roots, so a local's inferred type
/// still wins; this catches values whose type lives on the declaration rather
/// than at the use site.
///
/// Binds the root to its SPECIFIC in-scope declaration first: a local/field is
/// declared in an enclosing scope, so `{scope}.{name}` — derived from the use
/// site's owning symbol, innermost out — names the exact declaration whose type
/// to read, not a same-named value elsewhere in the project. `source_qname` is
/// the symbol owning the reference, so its own qname is the innermost scope (the
/// declaration is its child). Only when no in-scope binding exists do we fall
/// back to the first same-named value anywhere (an imported / ambient / top-level
/// value with no enclosing-scope qname relative to the use site).
fn value_root_type(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    name: &str,
    source_qname: &str,
) -> Option<TypeId> {
    let mut scope = source_qname;
    loop {
        let qn = if scope.is_empty() {
            name.to_string()
        } else {
            format!("{scope}.{name}")
        };
        if lookup
            .by_qualified_name(&qn)
            .is_some_and(|s| is_value_kind(&s.kind))
        {
            if let Some(id) = field_type_of(lookup, arena, &qn) {
                return Some(id);
            }
        }
        if scope.is_empty() {
            break;
        }
        scope = match scope.rfind('.') {
            Some(i) => &scope[..i],
            None => "",
        };
    }
    for cand in lookup.by_name(name) {
        if is_value_kind(&cand.kind) {
            if let Some(id) = field_type_of(lookup, arena, &cand.qualified_name) {
                return Some(id);
            }
        }
    }
    None
}

/// The interned field/property type of `qname`: the `field_type_id` if present,
/// else the string accessor interned at this boundary.
fn field_type_of(lookup: &dyn SymbolLookup, arena: &TypeArena, qname: &str) -> Option<TypeId> {
    if let Some(id) = lookup.field_type_id(qname) {
        return Some(id);
    }
    lookup
        .field_type_name(qname)
        .map(|s| arena.intern_type_str(s))
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
fn callee_return_type(lookup: &dyn SymbolLookup, arena: &TypeArena, name: &str) -> Option<TypeId> {
    let callee = lookup
        .by_name(name)
        .into_iter()
        .find(|s| is_callable(&s.kind))?;
    if let Some(id) = lookup.return_type_id(&callee.qualified_name) {
        return Some(id);
    }
    lookup
        .return_type_name(&callee.qualified_name)
        .map(|s| arena.intern_type_str(s))
}

/// Attach a segment's in-source type arguments to a freshly-interned bare head
/// that didn't already carry its own. A declared annotation reaches the chain
/// pre-split — `repo: Repository<User>` as head `Repository` + args `[User]` —
/// so the args must be reattached, else generic substitution sees no arguments.
fn with_segment_args(arena: &TypeArena, id: TypeId, type_args: &[String]) -> TypeId {
    if type_args.is_empty() {
        return id;
    }
    match arena.get(id) {
        Type::Class(_) => {
            let args = type_args.iter().map(|a| arena.intern_type_str(a)).collect();
            arena.intern(Type::Apply { base: id, args })
        }
        // Already an application (or another structural type) — the inline args
        // win and the segment's split args are redundant.
        _ => id,
    }
}

/// `true` when `kind` names something a call can root on — a free function or a
/// method whose return type carries the chain forward.
fn is_callable(kind: &str) -> bool {
    matches!(kind, "function" | "method")
}

#[cfg(test)]
#[path = "chain_tests.rs"]
mod tests;
