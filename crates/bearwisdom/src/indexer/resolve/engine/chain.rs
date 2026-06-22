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
    FileContext, RefContext, Symbol, SymbolInfo, SymbolLookup, RESOLVED_CONFIDENCE,
};
use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::types::{AliasTarget, SegmentKind};

use super::alias;
use super::support::import_scoped_package_id;

/// Strategy tag for a member-chain bind produced by the new engine.
const STRATEGY: &str = "rule_chain";

/// Upper bound on supertype-chain climbing when locating an inherited member.
const MAX_SUPERTYPE_DEPTH: usize = 8;

/// Resolve a member chain to its final segment's symbol. `None` when the root
/// can't be typed or a hop has no matching member — an honestly-unresolved
/// chain, diagnosable to the segment where the walk stopped.
pub fn bind_member_access(
    ref_ctx: &RefContext,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
) -> Option<SymbolInfo> {
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

    // The receiver threads a TypeId AND the symbol id of the declaration that
    // type names, when known. The id is the identity spine: a member step keys
    // on `members_of_id` so two declarations sharing a qname string stay
    // distinct, and the supertype climb is id-keyed. The id is `None` for a head
    // with no indexed declaration (external/ambient/string-parsed), and the walk
    // falls back to the qname-string member lookup there.
    let root = resolve_root(ref_ctx, file_ctx, lookup, arena, &chain.segments[0])?;
    let mut current = expand_receiver(root, lookup, arena);
    let last = chain.segments.len() - 1;

    for (i, seg) in chain.segments.iter().enumerate().skip(1) {
        let member = lookup_member_on(lookup, arena, current, &seg.name, &|_kind| true)?;
        if i == last {
            // The final member's yield type (with the receiver's type arguments
            // substituted) records x's type for `const x = a.b.c()` so a later
            // `x.method()` can root on it — forward inference compounds.
            let resolved_yield_type =
                yield_through(lookup, arena, &member, seg.is_call, current.ty);
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
        // Pin the new receiver's id using the member's package context so the
        // chain stays anchored to the package the use site established rather
        // than falling back to a first-winner by_qname re-search.
        let yielded = yield_through(lookup, arena, &member, seg.is_call, current.ty)?;
        current = expand_receiver(yielded_receiver(lookup, arena, yielded, member.package_id), lookup, arena);
    }
    None
}

/// A chain receiver: the type a segment evaluates to, plus the symbol id of the
/// declaration that type names when one is indexed. The id is the identity spine
/// of the walk — member lookup and the supertype climb key on it so two
/// declarations sharing a qname string never collide.
#[derive(Clone, Copy)]
pub(crate) struct Receiver {
    ty: TypeId,
    id: Option<i64>,
}

impl Receiver {
    /// A receiver whose declaration is known by id.
    fn new(ty: TypeId, id: i64) -> Self {
        Self { ty, id: Some(id) }
    }

    /// A receiver typed but not yet bound to a declaration id; the id is
    /// recovered from the type's head when `expand_receiver` runs.
    fn untyped(ty: TypeId) -> Self {
        Self { ty, id: None }
    }
}

/// Expand a receiver's type through any type alias, then resolve the declaration
/// id for the (possibly rewritten) head. When alias expansion changes the head,
/// the receiver names a different declaration — re-derive the id from the new
/// head via `by_qualified_name`. When the head is unchanged (no alias was
/// applied), the caller's established id is more specific than a first-winner
/// re-search: keep it and only fall back to `by_qualified_name` when no id was
/// carried. This prevents a same-qname collision in another package from
/// overwriting an id that was pinned at the use site or by prior-hop package
/// context.
fn expand_receiver(recv: Receiver, lookup: &dyn SymbolLookup, arena: &TypeArena) -> Receiver {
    let pre_head = head_qname(arena, recv.ty);
    let ty = alias::expand(recv.ty, lookup, arena);
    let post_head = head_qname(arena, ty);
    let id = if pre_head != post_head {
        // Alias rewrote the head — re-derive from the new head, fall back to
        // the carried id when the new head has no indexed declaration.
        head_symbol_id(arena, lookup, ty).or(recv.id)
    } else {
        // Head unchanged — the caller's id is the authoritative identity; the
        // by_qname first-winner is only a fallback for id-less (external/ambient)
        // receivers.
        recv.id.or_else(|| head_symbol_id(arena, lookup, ty))
    };
    Receiver { ty, id }
}

/// The symbol id of the declaration a type's nominal head names, resolved
/// through `by_qualified_name`. `None` for a head with no indexed declaration
/// (external/ambient/string-parsed) — the walk then falls back to qname-string
/// member lookup.
fn head_symbol_id(arena: &TypeArena, lookup: &dyn SymbolLookup, ty: TypeId) -> Option<i64> {
    let head = head_qname(arena, ty)?;
    lookup.by_qualified_name(&head).map(|s| s.id)
}

/// Build a `Receiver` for the type a member yielded, pinning the declaration id
/// from the member's package context when possible. A same-qname type in the
/// member's package is preferred over `by_qualified_name`'s first-winner, so the
/// chain stays anchored to the package the use site established. Falls back to
/// `by_qualified_name` when no package id is recorded on the member or when no
/// same-package declaration exists (external/ambient heads).
fn yielded_receiver(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    ty: TypeId,
    member_package_id: Option<i64>,
) -> Receiver {
    let id = head_symbol_id_preferring_package(arena, lookup, ty, member_package_id);
    Receiver { ty, id }
}

/// The symbol id of the declaration a type's head names, preferring a
/// same-package declaration over the first-winner when `preferred_pkg` is known.
/// Falls back to `by_qualified_name` (first-winner) for external/ambient heads
/// that have no indexed declaration in the preferred package.
fn head_symbol_id_preferring_package(
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    ty: TypeId,
    preferred_pkg: Option<i64>,
) -> Option<i64> {
    let head = head_qname(arena, ty)?;
    let simple = head.rsplit('.').next().unwrap_or(&head);
    if let Some(pkg) = preferred_pkg {
        for s in lookup.by_name(simple) {
            if s.qualified_name == head && s.package_id == Some(pkg) {
                return Some(s.id);
            }
        }
    }
    lookup.by_qualified_name(&head).map(|s| s.id)
}

/// Find `member` on a receiver, preferring the identity path: when the receiver
/// is bound to a declaration id, climb its supertypes by id via
/// `lookup_member_by_id`; otherwise fall back to the qname-string climb. The
/// id path is what keeps two same-qname receiver types apart.
fn lookup_member_on(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    recv: Receiver,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
) -> Option<Symbol> {
    lookup_member_on_bounded(lookup, arena, recv, member, accept, MAX_MAPPED_DEPTH)
}

/// Upper bound on mapped-type source hops — a mapped type whose source is itself
/// a mapped type chains here. Bounds the rare nesting and guards a cyclic alias.
const MAX_MAPPED_DEPTH: usize = 6;

/// `lookup_member_on` with a mapped-type recursion budget. After the direct and
/// supertype-climb lookups miss, a receiver that is a MAPPED type
/// (`{ [K in keyof T]: V }` — `Override<A, B>`, `BoundFunctions<Q>`) resolves the
/// member on its SOURCE object: the mapped type's keys ARE the source's keys, so
/// `mapped.member` IS `source.member`. The source param is bound to the
/// receiver's applied type argument, then the member walk recurses on it.
fn lookup_member_on_bounded(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    recv: Receiver,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
    depth: usize,
) -> Option<Symbol> {
    if let Some(id) = recv.id {
        if let Some(m) = lookup_member_by_id(lookup, id, member, accept) {
            return Some(m);
        }
    }
    let head = head_qname(arena, recv.ty)?;
    if let Some(m) = lookup_member(lookup, &head, member, accept) {
        return Some(m);
    }
    if depth == 0 {
        return None;
    }
    // Intersection alias `A & B & {…}` carries every member of every branch (TS
    // `&` semantics). After the direct / id / supertype climbs miss, resolve the
    // member on each NAMED branch — anonymous object branches are already
    // flattened into the alias's own members, so only the named heads remain.
    if let Some(m) = lookup_member_on_intersection(lookup, arena, &head, member, accept, depth) {
        return Some(m);
    }
    // Union alias `A | B | …`: TS union member access is valid only for members
    // present on EVERY arm — typically all arms extend a shared base that
    // declares the member. Resolve it on each arm; the shared declaration is the
    // result.
    if let Some(m) = lookup_member_on_union(lookup, arena, &head, member, accept, depth) {
        return Some(m);
    }
    // Primitive-name capitalization: a head like `"string"` / `"number"` /
    // `"boolean"` has no members keyed under the lowercase name, but the
    // nominal wrapper class (`"String"`, `"Number"`, `"Boolean"`) carries the
    // full member index. When the lowercase head has no indexed members, try
    // its capitalized form. This is universally sound — a head with an
    // UPPERCASE first letter is already handled by the ordinary member walk
    // above; only a lowercase-first head that looks like a built-in primitive
    // triggers this. A capitalized probe that finds nothing is a no-op.
    if let Some(m) = lookup_member_on_capitalized_primitive(lookup, &head, member, accept) {
        return Some(m);
    }
    if let Some(source_ty) = mapped_source_type(lookup, arena, recv.ty, &head) {
        let source_recv = expand_receiver(Receiver::untyped(source_ty), lookup, arena);
        // A mapped source that resolves back to the mapped type itself makes no
        // progress — skip rather than spin to the depth bound.
        if head_qname(arena, source_recv.ty).as_deref() != Some(head.as_str()) {
            if let Some(m) =
                lookup_member_on_bounded(lookup, arena, source_recv, member, accept, depth - 1)
            {
                return Some(m);
            }
        }
    }
    // The mapped source is an UNBOUND parameter: it falls back to its declared
    // default (a `typeof <namespace>`) whose keys are the mapped object's members,
    // resolved through the receiver's wildcard re-export closure.
    lookup_member_via_unbound_mapped_source(lookup, arena, recv.ty, &head, member, accept)
}

/// When `head` begins with a lowercase letter, probe the same name with its
/// first letter uppercased. Covers the primitive-to-nominal promotion for
/// languages (TypeScript, Java, Kotlin, Swift, …) where lowercase primitive
/// names (`string`, `int`, `boolean`) have a corresponding boxed/nominal
/// class (`String`, `Int`, `Boolean`) that carries the member index.
///
/// Only fires when the regular lowercase lookup already missed — this is a
/// last-resort fallback before the mapped-source hop, not a pre-pass.
/// Returns `None` when the head is already uppercase or the capitalized probe
/// finds no member.
fn lookup_member_on_capitalized_primitive(
    lookup: &dyn SymbolLookup,
    head: &str,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
) -> Option<Symbol> {
    let first = head.chars().next()?;
    if !first.is_lowercase() {
        return None;
    }
    let capitalized = {
        let mut s = String::with_capacity(head.len());
        s.extend(first.to_uppercase());
        s.push_str(&head[first.len_utf8()..]);
        s
    };
    lookup_member(lookup, &capitalized, member, accept)
}

/// Resolve `member` on the named branches of an intersection alias. A branch is
/// the head name of an `&` member (`BoundFunctions` for `BoundFunctions<Q> & {…}`);
/// anonymous object branches contribute no name and are skipped (their members are
/// flattened onto the alias itself). Each branch name is resolved to its
/// declaration(s) by simple name, then the member walk recurses by symbol id so a
/// branch shared across packages stays distinct and the branch's own supertypes
/// climb. Returns the first branch that carries `member`. `None` when `head` is
/// not an intersection alias or no branch carries the member.
fn lookup_member_on_intersection(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    head: &str,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
    depth: usize,
) -> Option<Symbol> {
    let branches = match lookup.alias_target(head)? {
        AliasTarget::Intersection(branches)
        | AliasTarget::IntersectionMapped { branches, .. } => branches.clone(),
        _ => return None,
    };
    for branch in &branches {
        if branch.is_empty() || branch == head {
            continue;
        }
        for cand in lookup.types_by_name(branch).iter() {
            let recv = expand_receiver(
                Receiver::new(arena.class(&cand.qualified_name), cand.id),
                lookup,
                arena,
            );
            // A branch that resolves back to the intersection itself makes no
            // progress — skip rather than recurse to the depth bound.
            if head_qname(arena, recv.ty).as_deref() == Some(head) {
                continue;
            }
            if let Some(m) = lookup_member_on_bounded(lookup, arena, recv, member, accept, depth - 1)
            {
                return Some(m);
            }
        }
    }
    None
}

/// Resolve `member` on a UNION alias `A | B | …`. TS union member access is
/// valid only for members present on EVERY arm, so the member resolves on the
/// union iff every named branch carries it — the canonical shape is a tagged
/// result union whose arms all `extends` a common base that declares the member.
/// Each branch head is resolved to its declaration(s) and the member walk
/// recurses by symbol id (climbing the branch's supertypes); the first arm's
/// resolution is returned once every arm has agreed it carries the member.
/// `None` when `head` is not a union alias, a branch is unnameable (a
/// primitive/literal arm that cannot carry the member), or any arm lacks it.
fn lookup_member_on_union(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    head: &str,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
    depth: usize,
) -> Option<Symbol> {
    let branches = match lookup.alias_target(head)? {
        AliasTarget::Union(branches) => branches.clone(),
        _ => return None,
    };
    if branches.is_empty() {
        return None;
    }
    let mut resolved: Option<Symbol> = None;
    for branch in &branches {
        if branch.is_empty() || branch == head {
            // A primitive/literal/self arm cannot carry the member; union access
            // requires it on every arm, so the access is invalid.
            return None;
        }
        let mut branch_hit: Option<Symbol> = None;
        for cand in lookup.types_by_name(branch).iter() {
            let recv = expand_receiver(
                Receiver::new(arena.class(&cand.qualified_name), cand.id),
                lookup,
                arena,
            );
            // A branch that resolves back to the union itself makes no progress.
            if head_qname(arena, recv.ty).as_deref() == Some(head) {
                continue;
            }
            if let Some(m) = lookup_member_on_bounded(lookup, arena, recv, member, accept, depth - 1)
            {
                branch_hit = Some(m);
                break;
            }
        }
        match branch_hit {
            None => return None,
            Some(m) => {
                if resolved.is_none() {
                    resolved = Some(m);
                }
            }
        }
    }
    resolved
}

/// The source object type of a mapped alias `{ [K in keyof Src]: … }`, with the
/// mapped param bound to the receiver's applied type argument: `Override<A, B>`
/// (params `[A, B]`, source `A`) with receiver `Override<MutationObserverResult,
/// …>` yields `MutationObserverResult`. A source naming a concrete type rather
/// than a param is interned directly. `None` when `head` is not a mapped alias
/// or the mapped capture recorded no source.
fn mapped_source_type(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    ty: TypeId,
    head: &str,
) -> Option<TypeId> {
    let source = match lookup.alias_target(head) {
        Some(AliasTarget::Mapped { source, .. })
        | Some(AliasTarget::IntersectionMapped { source, .. })
            if !source.is_empty() =>
        {
            source.clone()
        }
        _ => return None,
    };
    let args = apply_args(arena, ty);
    let params = lookup
        .generic_params(head)
        .map(|p| p.to_vec())
        .unwrap_or_default();
    if let Some(pos) = params.iter().position(|p| *p == source) {
        if let Some(&arg) = args.get(pos) {
            return Some(arg);
        }
        // Source names a parameter with NO applied argument — unbound. The bare
        // parameter name carries no members; its default (a `typeof <namespace>`)
        // is resolved by `lookup_member_via_unbound_mapped_source` through the
        // receiver's re-export closure instead.
        return None;
    }
    Some(arena.intern_type_str(&source))
}

/// Resolve `member` on a mapped alias whose source parameter is UNBOUND — the
/// receiver supplied no type argument, so the parameter falls back to its
/// declared default. The canonical shape is testing-library's `RenderResult`:
/// `{ [P in keyof Q]: BoundFunction<Q[P]> }` with `Q extends Queries = typeof
/// queries`. `keyof Q`'s keys are the query functions (`getByText`, …) of the
/// value namespace `queries`, and the receiver type's package re-exports that
/// whole namespace (`export * from '@testing-library/dom'`). Walk the receiver
/// declaration's wildcard re-exports and resolve `member` by qualified name in
/// each re-exported module.
///
/// Gated to: a Mapped / IntersectionMapped alias whose source is one of the
/// type's own generic parameters with NO applied argument (a bound source is
/// handled by `mapped_source_type`), declared in an EXTERNAL file (the namespace
/// + wholesale re-export shape this targets only occurs in library `.d.ts`).
/// `None` outside that shape or when no re-exported module carries the member.
fn lookup_member_via_unbound_mapped_source(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    ty: TypeId,
    head: &str,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
) -> Option<Symbol> {
    let source = match lookup.alias_target(head)? {
        AliasTarget::Mapped { source, .. } | AliasTarget::IntersectionMapped { source, .. }
            if !source.is_empty() =>
        {
            source.clone()
        }
        _ => return None,
    };
    let params = lookup.generic_params(head).unwrap_or(&[]);
    let pos = params.iter().position(|p| *p == source)?;
    // A source param with a CONCRETE applied argument is bound — `mapped_source_type`
    // resolved it already. But a self-applied return type echoes its own parameters
    // as arguments (`render(): RenderResult<Q, Container, BaseElement>`), so the arg
    // at `pos` is the parameter name itself — still unbound, still defaulted. Treat
    // an argument whose head is one of the type's own parameters as unbound.
    let bound = apply_args(arena, ty)
        .get(pos)
        .and_then(|&a| head_qname(arena, a))
        .is_some_and(|h| !params.iter().any(|p| *p == h));
    if bound {
        return None;
    }
    let decl_file = lookup.by_qualified_name(head)?.file_path.clone();
    if !lookup.is_external_file(&decl_file) {
        return None;
    }
    for (orig, module) in lookup.reexports_from(&decl_file) {
        if orig.as_str() != "*" {
            continue;
        }
        let want = format!("{module}.{member}");
        for cand in lookup.by_name(member).iter() {
            if cand.qualified_name == want && accept(&cand.kind) {
                return Some(cand.clone());
            }
        }
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

/// Resolve `member` on the declaration `type_id`, climbing its supertypes up to
/// `MAX_SUPERTYPE_DEPTH` by SYMBOL ID. Both the member index (`members_of_id`)
/// and the supertype climb (`parent_class_id`) key on identity, so a receiver
/// whose qname is shared by an unrelated type in another package binds members
/// only from THIS declaration and climbs only ITS recorded base — never a
/// first-wins qname collision.
pub(crate) fn lookup_member_by_id(
    lookup: &dyn SymbolLookup,
    type_id: i64,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
) -> Option<Symbol> {
    // Breadth-first over the supertype DAG: a type can extend/implement several
    // supertypes, and the member may be declared on any of them, so every direct
    // parent is followed (not a single linear chain). A visited set prevents
    // re-walking a diamond, and the depth bound caps the climb.
    let mut visited: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut frontier = vec![type_id];
    for _ in 0..MAX_SUPERTYPE_DEPTH {
        let mut next: Vec<i64> = Vec::new();
        for cur in frontier {
            if !visited.insert(cur) {
                continue;
            }
            for m in lookup.members_of_id(cur) {
                if m.name == member && accept(&m.kind) {
                    return Some(m.clone());
                }
            }
            next.extend(lookup.parent_class_ids(cur));
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
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
///
/// Callable-property fallback: when `is_call=true` and no explicit return type
/// is recorded, the member may be a property whose declared type is a function
/// signature (e.g. `fn: () => Mock<T>`). In that case the field type is
/// returned unchanged; `yield_through` peels the `Type::Function` wrapper to
/// extract the call result.
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
        if let Some(s) = lookup.return_type_name(member_qname) {
            return Some(arena.intern_type_str(s));
        }
        // No explicit return type — the member's declared (field) type may be
        // callable. An inline function type (`fn: () => Mock`) is returned as-is
        // for `yield_through` to peel its `Type::Function` wrapper. A field type
        // that NAMES a function/method symbol (a property typed `typeof someFn`)
        // interns as a nominal head, not a `Type::Function`, so resolve it to
        // the named function's own return type here.
        let ft = lookup
            .field_type_id(member_qname)
            .or_else(|| lookup.field_type_name(member_qname).map(|s| arena.intern_type_str(s)))?;
        if let Some(head) = head_qname(arena, ft) {
            if let Some(rt) = callable_named_return(lookup, arena, &head) {
                return Some(rt);
            }
        }
        return Some(ft);
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
///
/// Fluent-chain rebind: when the substituted yield type's head is `"this"` or
/// `"Self"`, the member is a fluent step that returns the receiver itself.
/// Substitute the receiver type directly so the next member lookup stays on
/// the receiver's type rather than dead-ending on a nominal `Class("this")`
/// that has no members.
///
/// Callable-property unwrap: when `is_call=true` and the member's declared
/// yield type is a function type (`Type::Function { return_ }`), the call
/// result is the function's own return type — a property whose declared type
/// is a call signature (`fn: () => Mock<T>`) yields `Mock<T>` when called.
fn yield_through(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    member: &Symbol,
    is_call: bool,
    receiver: TypeId,
) -> Option<TypeId> {
    let raw = member_yield_type(lookup, arena, &member.qualified_name, is_call)?;
    // Callable-property unwrap: if the raw yield is a function type and the
    // member is being called, peel off one call layer to get the return type.
    // This covers `fn: () => Mock<T>` properties: the field type interns as
    // `Type::Function { return_: Mock<T> }`; calling the property yields
    // `Mock<T>`, not the function-type descriptor itself.
    let raw = if is_call {
        match arena.get(raw) {
            crate::type_checker::core::types::Type::Function { return_, .. } => return_,
            _ => raw,
        }
    } else {
        raw
    };
    let substituted = substitute_through(lookup, arena, raw, receiver);
    // Fluent-chain rebind: `this`/`Self` head means "return the receiver".
    if is_self_head(arena, substituted) {
        return Some(receiver);
    }
    Some(substituted)
}

/// `true` when the type's nominal head is the `this` or `Self` keyword —
/// the fluent-return convention across TypeScript (`this`), Rust (`Self`),
/// Swift (`Self`), and Kotlin (`this`). When a member's yield type has this
/// head, the chain advances to the RECEIVER's type instead.
fn is_self_head(arena: &TypeArena, id: TypeId) -> bool {
    match head_qname(arena, id) {
        Some(h) => h == "this" || h == "Self",
        None => false,
    }
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
///
/// Returns a `Receiver`: the root type plus, where a SPECIFIC declaration is in
/// hand (the bare type name resolved through `types_by_name`, the enclosing type
/// for `this`/`self`), that declaration's symbol id. Cases that produce a type
/// EXPRESSION (a local's inferred type, a declared annotation, a value's field
/// type, a callee return) leave the id unbound; `expand_receiver` recovers it
/// from the type's head. The id is what keeps a same-named receiver type in one
/// package distinct from another's during the member walk.
fn resolve_root(
    ref_ctx: &RefContext,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    seg: &crate::types::ChainSegment,
) -> Option<Receiver> {
    // Prefer the TypeId cache: `local_type_id` returns the TypeId that was
    // stored directly by `record_local_type_id`, preserving the exact type
    // variant (Primitive, Optional, Generic) without a format/intern round-trip.
    // Fall back to the String cache and intern once at this boundary when no
    // TypeId binding exists (e.g. bindings recorded via the String path or by
    // synthetic test doubles that only implement `local_type`).
    // A binding typed `ReturnType<typeof fn>` (or any return-type-extraction alias
    // of that shape) roots on `fn`'s return type, resolved in this file's import
    // scope so the right overload is chosen. Applies to both the TypeId-cached and
    // String-cached local bindings.
    if let Some(id) = lookup.local_type_id(&seg.name) {
        if let Some(r) = resolve_return_type_extraction(id, lookup, arena, file_ctx) {
            return Some(Receiver::untyped(r));
        }
        return Some(Receiver::untyped(id));
    }
    if let Some(ty) = lookup.local_type(&seg.name) {
        let id = arena.intern_type_str(&ty);
        if let Some(r) = resolve_return_type_extraction(id, lookup, arena, file_ctx) {
            return Some(Receiver::untyped(r));
        }
        return Some(Receiver::untyped(id));
    }
    if let Some(ty) = &seg.declared_type {
        return Some(Receiver::untyped(with_segment_args(
            arena,
            arena.intern_type_str(ty),
            &seg.type_args,
        )));
    }
    if matches!(seg.kind, SegmentKind::SelfRef) {
        // `this`/`self` roots on the enclosing type — the one declaration whose
        // members the chain walks. Bind its id so an inherited-member climb keys
        // on identity, not the enclosing type's qname string.
        let enc_qname = lookup.enclosing_type_qname(&ref_ctx.source_symbol.qualified_name)?;
        let ty = arena.class(enc_qname);
        let id = lookup.by_qualified_name(enc_qname).map(|s| s.id);
        return Some(Receiver { ty, id });
    }
    if seg.is_call {
        if let Some(ty) = callee_return_type(lookup, arena, file_ctx, &seg.name) {
            return Some(Receiver::untyped(ty));
        }
        // The callee is not a callable declaration (function/method) but may be
        // a VALUE whose declared type is a callable interface — `const expect:
        // ExpectStatic` where `ExpectStatic` carries a call signature. Calling
        // it yields the call signature's return, not the interface type itself,
        // so this must precede the value-root fallthrough below.
        if let Some(ty) = call_value_root_type(
            lookup,
            arena,
            &seg.name,
            &ref_ctx.source_symbol.qualified_name,
        ) {
            return Some(Receiver::untyped(ty));
        }
    }
    // Import-of-value / typed-value root: a value (a `declare const`, an
    // imported binding, a typed field) whose declaration carries a type roots
    // the chain on that type — `initTRPC.create()` roots on `initTRPC`'s builder
    // type even though `initTRPC` is not itself a type name.
    if let Some(ty) =
        value_root_type(lookup, arena, &seg.name, &ref_ctx.source_symbol.qualified_name)
    {
        return Some(Receiver::untyped(ty));
    }
    // Bare type name used as a static-access / construction root. When the same
    // name is declared in several sibling workspace packages, prefer the
    // declaration in the package the use site imports the name from; otherwise
    // fall back to the first same-named type. The resolved `Symbol` IS the
    // receiver's declaration, so bind its id directly rather than round-tripping
    // its qname back through `by_qualified_name`.
    let candidates = lookup.types_by_name(&seg.name);
    let s = import_scoped_package_id(file_ctx, lookup, &seg.name)
        .and_then(|pkg| candidates.iter().find(|c| c.package_id == Some(pkg)))
        .or_else(|| candidates.first())?;
    let ty = with_segment_args(arena, arena.class(&s.qualified_name), &seg.type_args);
    Some(Receiver::new(ty, s.id))
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
///
/// When the resulting type's nominal head names a VALUE rather than a type — the
/// shape the extractor lowers `const x: typeof import('m')['k']` to, where the
/// field type is the bare exported name `k` and `k` is a value the module
/// exports — the value's *own* declared type is the real receiver. Follow that
/// indirection so the chain roots on the referenced export's type, not on the
/// (non-existent) type named by the export's identifier.
fn field_type_of(lookup: &dyn SymbolLookup, arena: &TypeArena, qname: &str) -> Option<TypeId> {
    let id = raw_field_type_of(lookup, arena, qname)?;
    Some(deref_value_typed(lookup, arena, id, qname))
}

/// The interned field/property type of `qname` without value-indirection
/// following: the `field_type_id` if present, else the string accessor interned
/// at this boundary.
fn raw_field_type_of(lookup: &dyn SymbolLookup, arena: &TypeArena, qname: &str) -> Option<TypeId> {
    if let Some(id) = lookup.field_type_id(qname) {
        return Some(id);
    }
    lookup
        .field_type_name(qname)
        .map(|s| arena.intern_type_str(s))
}

/// Upper bound on value-indirection hops when a field type names a value whose
/// own declared type is the real receiver. Bounds the rare re-export-shim chain
/// and prevents a self-referential field type from looping.
const MAX_VALUE_TYPE_DEREF: usize = 4;

/// Follow a field type whose nominal head names a VALUE through to the type that
/// declares the members. Two indirections collapse here:
///
///   - A head whose own declaration is a member-less value re-export shim — a
///     `vitest.ExpectStatic` variable with no members, re-exporting an interface
///     of the same simple name declared under a different qname — re-roots onto
///     that same-simple-name type declaration, where the members are keyed.
///   - A head that names no type but names a value whose own field type differs
///     (`const x: typeof import('m')['k']` lowered to the bare export name)
///     follows that value's declared type.
///
/// Stops at a type head that already declares members, a value with no field
/// type, a fixpoint (the value's field type names itself), or
/// `MAX_VALUE_TYPE_DEREF` hops. `origin_qname` is the symbol the original field
/// type was read from; it is excluded from the value lookup so a self-typed shim
/// doesn't loop on itself.
fn deref_value_typed(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    id: TypeId,
    origin_qname: &str,
) -> TypeId {
    let mut current = id;
    let mut seen_qname = origin_qname.to_string();
    for _ in 0..MAX_VALUE_TYPE_DEREF {
        let Some(head) = head_qname(arena, current) else {
            return current;
        };
        // The maps are simple-name keyed; a qualified head (`vitest.ExpectStatic`)
        // probes them by its trailing segment.
        let simple = head.rsplit('.').next().unwrap_or(&head);
        // A nominal head whose declaration is a member-bearing type is already
        // the receiver. When a same-simple-name type exists but the head's own
        // declaration is a member-less value re-export shim, advance the receiver
        // onto the type declaration that actually holds the members — its qname
        // differs from the shim's, so the member walk would otherwise dead-end on
        // the member-less value.
        if let Some(type_decl) = receiver_type_for_head(lookup, &head, simple) {
            if type_decl.qualified_name == head {
                return current;
            }
            return arena.class(&type_decl.qualified_name);
        }
        // The head names no type; if it names a value (other than the symbol we
        // just came from) whose own field type differs, that value's type is the
        // real receiver.
        let Some(value) = lookup
            .by_name(simple)
            .into_iter()
            .find(|s| is_value_kind(&s.kind) && s.qualified_name != seen_qname)
        else {
            return current;
        };
        let Some(next) = raw_field_type_of(lookup, arena, &value.qualified_name) else {
            return current;
        };
        if next == current {
            return current;
        }
        current = next;
        seen_qname = value.qualified_name.clone();
    }
    current
}

/// The type declaration a head should root on: the same-simple-name type whose
/// members the chain walks. `None` when no type shares the head's simple name —
/// the caller then tries value indirection.
///
/// When the head's OWN declaration is the type (a normal member-bearing type
/// name), that declaration is returned and the caller leaves the receiver
/// unchanged. When the head names a member-less value re-export shim and a
/// distinct same-simple-name type declares the members, that type is returned so
/// the caller re-roots onto it. A type candidate that carries members is
/// preferred over a member-less one, so the shim's own member-less value (which
/// can also be type-kind in some shapes) never shadows the real declaration.
fn receiver_type_for_head(
    lookup: &dyn SymbolLookup,
    head: &str,
    simple: &str,
) -> Option<Symbol> {
    let candidates = lookup.types_by_name(simple).to_vec();
    if candidates.is_empty() {
        return None;
    }
    // A type candidate whose qname equals the head and bears members is the head
    // itself as a member-bearing type — keep it (caller leaves the receiver as-is).
    if let Some(exact) = candidates
        .iter()
        .find(|c| c.qualified_name == head && type_has_members(lookup, c))
    {
        return Some(exact.clone());
    }
    // Otherwise prefer a member-bearing type declaration (the interface that holds
    // the call signature / members), falling back to the first type candidate.
    candidates
        .iter()
        .find(|c| type_has_members(lookup, c))
        .or_else(|| candidates.first())
        .cloned()
}

/// `true` when the type declaration `sym` has at least one indexed member —
/// either id-keyed (`members_of_id`) or qname-keyed (`members_of`).
fn type_has_members(lookup: &dyn SymbolLookup, sym: &Symbol) -> bool {
    !lookup.members_of_id(sym.id).is_empty() || !lookup.members_of(&sym.qualified_name).is_empty()
}

/// `true` when `kind` names a value whose declared type can root a chain.
fn is_value_kind(kind: &str) -> bool {
    matches!(
        kind,
        "variable" | "constant" | "const" | "field" | "property" | "parameter"
    )
}

/// Type a call root whose callee is a VALUE of a callable-interface type, by the
/// return of that interface's call signature: `const expect: ExpectStatic` where
/// `ExpectStatic` carries `(x): Assertion` types `expect(x)` as `Assertion`. The
/// extractor synthesises that call signature as a member named `call` on the
/// interface; this types the value to its declared interface, then yields the
/// `call` member's return with the receiver's type arguments substituted — the
/// same member-walk and substitution every chain hop uses, so any callable
/// interface's value types its call result generically.
fn call_value_root_type(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    name: &str,
    source_qname: &str,
) -> Option<TypeId> {
    let value_ty = value_root_type(lookup, arena, name, source_qname)?;
    let recv = expand_receiver(Receiver::untyped(value_ty), lookup, arena);
    let call = lookup_member_on(lookup, arena, recv, CALL_SIGNATURE_MEMBER, &|_kind| true)?;
    yield_through(lookup, arena, &call, true, recv.ty)
}

/// The name the extractor synthesises for an interface's call signature
/// (`interface F { (x): R }`), surfaced as a member so a value of that interface
/// type yields `R` when called.
const CALL_SIGNATURE_MEMBER: &str = "call";

/// Type a call root by the callee's return type: `makeRepo()` where
/// `makeRepo(): Repository<User>` types the chain head as `Repository<User>`.
///
/// When several callable declarations share `name` across sibling packages,
/// prefer the one in the package the use site imports `name` from, so the chain
/// heads on THAT declaration's return type. Without an import attribution, or
/// when no candidate sits in the imported package, the first callable wins. This
/// is the same import-scoped declaration pick the bare-type-name root uses; no
/// per-library knowledge enters here.
fn callee_return_type(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file_ctx: &FileContext,
    name: &str,
) -> Option<TypeId> {
    let candidates = lookup.by_name(name);
    let scoped = import_scoped_package_id(file_ctx, lookup, name).and_then(|pkg| {
        candidates
            .iter()
            .find(|s| is_callable(&s.kind) && s.package_id == Some(pkg))
    });
    let callee = match scoped {
        Some(c) => c,
        None => candidates.iter().find(|s| is_callable(&s.kind))?,
    };
    // Prefer the resolved callee's OWN return (id-keyed) over the qname slot,
    // which a same-named declaration in another package may have won.
    if let Some(id) = lookup.return_type_id_of(callee.id) {
        return Some(id);
    }
    if let Some(id) = lookup.return_type_id(&callee.qualified_name) {
        return Some(id);
    }
    lookup
        .return_type_name(&callee.qualified_name)
        .map(|s| arena.intern_type_str(s))
}

/// Resolve a return-type-extraction application — `ReturnType<typeof fn>`, and
/// any user alias of the same shape — to `fn`'s return type.
///
/// The shape is structural, not name-based (see `is_return_type_extraction`).
/// Applied to `typeof fn`, the checked type is `fn`'s value, so the result is
/// `fn`'s return type, resolved in the use site's import scope via
/// `callee_return_type` so the correct overload / package is selected. `None`
/// when `ty_str` is not such an application, or the callee or its return type
/// can't be resolved.
pub(crate) fn resolve_return_type_extraction(
    ty: TypeId,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file_ctx: &FileContext,
) -> Option<TypeId> {
    let head = head_qname(arena, ty)?;
    if !is_return_type_extraction(lookup.alias_target(&head)?) {
        return None;
    }
    let arg = apply_args(arena, ty).first().copied()?;
    let value = head_qname(arena, arg)?
        .strip_prefix("typeof ")
        .map(str::trim)?
        .to_string();
    typeof_value_return_type(&value, lookup, arena, file_ctx)
}

/// The return type of the function bound to `value` at the use site. The binding
/// is scoped to the module `value` is IMPORTED from (`import { render } from
/// '@testing-library/react'` → `@testing-library/react.render`), so an externally
/// imported callee resolves to the correct overload rather than a same-named
/// function in another package — `callee_return_type`'s name-only fallback (which
/// only package-scopes WORKSPACE imports) would otherwise pick a first-winner.
/// Falls back to that name-only resolution for a value with no matching import.
fn typeof_value_return_type(
    value: &str,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file_ctx: &FileContext,
) -> Option<TypeId> {
    for import in &file_ctx.imports {
        let names = import.imported_name == value || import.alias.as_deref() == Some(value);
        if !names {
            continue;
        }
        let Some(module) = import.module_path.as_deref() else {
            continue;
        };
        let qname = format!("{module}.{}", import.imported_name);
        if let Some(id) = lookup.return_type_id(&qname) {
            return Some(id);
        }
        if let Some(s) = lookup.return_type_name(&qname) {
            return Some(arena.intern_type_str(s));
        }
    }
    callee_return_type(lookup, arena, file_ctx, value)
}

/// A `Conditional` of the return-type-extraction shape: `T extends (...) => infer
/// R ? R : …`. Recognised structurally — `extends` ends in `=> infer <V>` and the
/// true branch is `<V>` — so it matches the lib `ReturnType<T>` and any alias
/// written the same way, regardless of name.
fn is_return_type_extraction(target: &AliasTarget) -> bool {
    let AliasTarget::Conditional {
        extends,
        true_branch,
        ..
    } = target
    else {
        return false;
    };
    let Some((_, infer_tail)) = extends.rsplit_once("=> infer ") else {
        return false;
    };
    let infer_var = infer_tail
        .trim_start()
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()
        .unwrap_or("");
    !infer_var.is_empty() && infer_var == true_branch.trim()
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

/// When a called member's field type NAMES a function/method symbol — the shape
/// a property typed `typeof someFn` produces — the call result is that named
/// function's return type. Prefers a qualified-name hit, then the first callable
/// of that simple name. `None` when `head` doesn't resolve to a callable.
fn callable_named_return(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    head: &str,
) -> Option<TypeId> {
    let callee_qname = lookup
        .by_qualified_name(head)
        .filter(|s| is_callable(&s.kind))
        .map(|s| s.qualified_name.clone())
        .or_else(|| {
            lookup
                .by_name(head)
                .iter()
                .find(|s| is_callable(&s.kind))
                .map(|s| s.qualified_name.clone())
        })?;
    if let Some(id) = lookup.return_type_id(&callee_qname) {
        return Some(id);
    }
    lookup
        .return_type_name(&callee_qname)
        .map(|s| arena.intern_type_str(s))
}

#[cfg(test)]
#[path = "chain_tests.rs"]
mod tests;
