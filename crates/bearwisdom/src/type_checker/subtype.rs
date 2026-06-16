// =============================================================================
// type_checker/subtype.rs — Conservative subtype check for conditional types
//
// Used by `expand_alias`'s `Conditional` arm to decide which branch a
// `T extends U ? X : Y` alias resolves to. The check is intentionally
// conservative: it returns `Some(true)` / `Some(false)` only when the
// answer is unambiguous, and `None` (undecidable) otherwise. Returning
// `None` makes the chain walker miss against the alias — which is the
// right outcome when we can't decide, since picking the wrong branch
// would replace a clean miss with a wrong resolution.
//
// What we recognise:
//   - Identity:   X extends X      → true
//   - Universal:  X extends any    → true (also `unknown`)
//   - Bottom:     never extends Y  → true (vacuously)
//   - Inheritance: X extends Y when X has Y in its class chain (capped
//                  to 10 hops; matches the chain walker's existing cap).
//   - Primitive disjointness: two different primitives → false.
//
// What we don't recognise (and return `None` for):
//   - Structural assignability across non-class types.
//   - Generic constraints with conditional or mapped types.
//   - `infer` clauses — they require a separate matcher.
//   - Union / intersection branch checks.
//
// The TypeId form (`is_assignable_to_typed_with`) adds one ADDITIONAL positive
// path on top of the nominal arms: a SHAPE-based check between two shape-bearing
// types (Class/Interface/Struct). A source structurally satisfies a target when
// it carries every one of the target's direct members with a matching name, a
// matching kind, AND a member type assignable in the right variance (method
// returns covariant, method params contravariant, field types covariant) — TS
// structural typing and Go implicit interface satisfaction. The structural arm
// is gated hard for soundness: it only ever turns an Unknown into Yes, never an
// inheritance/primitive answer into something looser, and it returns Yes ONLY
// when the full member set of both sides is enumerable, every target member is
// present with a matching kind, and both matched members carry recorded type
// data whose comparison is definitely assignable. A matched member with no
// recorded `SymbolTypeData` keeps the arm at Unknown — declaring assignable on
// name+kind alone (ignoring parameter / return types) is the unsound path this
// check exists to avoid.
// =============================================================================

use crate::indexer::resolve::legacy::{SymbolInfo, SymbolLookup};
use crate::type_checker::core::members::MembersIndex;
use crate::type_checker::core::symbol_types::SymbolTypeMap;
use crate::type_checker::core::symbol_view::SymbolView;
use crate::type_checker::core::types::{PrimKind, Type, TypeArena, TypeId};

/// TypeScript primitives the conservative branch uses to detect
/// disjoint primitive pairs (e.g. `"string" extends number ? ...`).
/// Kept narrow on purpose — additions here change branch selection
/// across every conditional alias in the index.
const PRIMITIVES: &[&str] = &[
    "string",
    "number",
    "boolean",
    "bigint",
    "symbol",
    "null",
    "undefined",
    "void",
];

/// Maximum hops to follow `parent_class_qname` when checking an
/// inheritance relationship. Matches the chain walker's own cap so
/// pathological cycles never block resolution.
const MAX_INHERITANCE_HOPS: u8 = 10;

/// Decide whether `check` is assignable to `extends` for the purposes
/// of conditional-type branch selection.
///
/// Returns:
/// - `Some(true)` — definitely assignable; caller picks the true branch.
/// - `Some(false)` — definitely not assignable; caller picks the false branch.
/// - `None` — undecidable; caller returns None (chain walker misses).
pub fn is_assignable_to(check: &str, extends: &str, lookup: &dyn SymbolLookup) -> Option<bool> {
    if check.is_empty() || extends.is_empty() {
        return None;
    }
    if check == extends {
        return Some(true);
    }
    // `any` and `unknown` are top types — every value is assignable to them.
    if matches!(extends, "any" | "unknown") {
        return Some(true);
    }
    // `never` is the bottom type — vacuously assignable to everything.
    if check == "never" {
        return Some(true);
    }
    // Walk the inheritance chain: `check` extends `extends` if `extends`
    // appears anywhere in `check`'s class ancestry. Cap at
    // MAX_INHERITANCE_HOPS to guard against malformed cycles, the same
    // way the chain walker caps its own ancestor walk.
    let mut ancestor = check.to_string();
    for _ in 0..MAX_INHERITANCE_HOPS {
        let Some(parent) = lookup.parent_class_qname(&ancestor) else {
            break;
        };
        if parent == extends {
            return Some(true);
        }
        if parent == ancestor {
            // Self-referential parent map — bail before looping.
            break;
        }
        ancestor = parent.to_string();
    }
    // Two different primitives are definitely disjoint. We don't try
    // any subtype reasoning between primitives (TS's `1 extends number`
    // case requires literal-vs-primitive widening machinery we don't
    // have); we only assert *non*-assignability between two primitives
    // that are different.
    if PRIMITIVES.contains(&check) && PRIMITIVES.contains(&extends) {
        return Some(false);
    }
    None
}

// ---------------------------------------------------------------------------
// TypeId form — Phase 2 of the engine pivot.
//
// The legacy string fn above remains for chain-walker callers still on the
// string path. New code working off `TypeArena` calls `is_assignable_to_typed`
// and matches on `SubtypeResult` directly. Both implementations share the
// same conservative semantics: only commit to Yes/No when the answer is
// unambiguous; return Unknown to let the caller fall through to a clean miss.
// ---------------------------------------------------------------------------

/// Three-valued result for the conservative subtype check.
///
/// `Unknown` is load-bearing: returning a guess (Yes/No) instead would let a
/// conditional alias pick a branch on shaky evidence and silently corrupt
/// downstream resolution. Callers translate Unknown to "miss" rather than
/// "wrong target."
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtypeResult {
    Yes,
    No,
    Unknown,
}

/// Public entry for the typed subtype check with no language primitive map.
/// Structural `Primitive` pairs are still compared; nominal primitive names
/// (`Class("number")`) are recognized only via [`is_assignable_to_typed_with`].
pub fn is_assignable_to_typed(
    source: TypeId,
    target: TypeId,
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    members: &MembersIndex,
    symbol_types: &SymbolTypeMap,
) -> SubtypeResult {
    is_assignable_to_typed_with(source, target, arena, lookup, members, symbol_types, &[])
}

/// Map a type to its primitive kind for disjointness checks. A structural
/// `Primitive` yields its kind directly; a nominal `Class` yields a kind only
/// when its qname appears in `prims` (a language's `primitive_mapping`). This
/// bridges the two ways a primitive annotation can be interned, so `string`
/// vs `number` is decided whether either side is `Primitive(..)` or `Class(..)`.
fn prim_kind_of(ty: &Type, prims: &[(&str, PrimKind)]) -> Option<PrimKind> {
    match ty {
        Type::Primitive(k) => Some(*k),
        Type::Class(q) => prims
            .iter()
            .find(|(n, _)| *n == q.as_str())
            .map(|(_, k)| *k),
        _ => None,
    }
}

/// TypeId form of `is_assignable_to`. Decides whether `source` is assignable
/// to `target` using only the type's structural shape in the arena plus the
/// supertype chain reachable through `SymbolLookup::parent_class_qname`.
///
/// Recognized:
/// - Identity (TypeId equality, integer comparison).
/// - Top types: target is `Class("any")` / `Class("unknown")`.
/// - Bottom: source is `Primitive(Never)` or `Class("never")`.
/// - `Optional<T>` target: source assignable when assignable to `T`.
/// - `Union` source: assignable when *every* branch is.
/// - `Union` target: assignable when *any* branch is.
/// - `Class` → `Class`: walk inheritance via parent_class_qname (capped to
///   `MAX_INHERITANCE_HOPS` matching the string form). When the nominal walk
///   finds no relationship, an ADDITIONAL structural arm decides Yes when the
///   source carries every direct member of the target with a matching name and
///   kind (see [`structurally_assignable`]).
/// - `Primitive` → `Primitive`: equal kinds → Yes, different kinds → No.
///
/// Everything else returns `Unknown`.
pub fn is_assignable_to_typed_with(
    source: TypeId,
    target: TypeId,
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    members: &MembersIndex,
    symbol_types: &SymbolTypeMap,
    prims: &[(&str, PrimKind)],
) -> SubtypeResult {
    assignable_inner(
        source,
        target,
        arena,
        lookup,
        members,
        symbol_types,
        prims,
        0,
    )
}

/// Maximum nesting of the structural member-type re-entry before the structural
/// arm bails to Unknown. A member's type check can re-enter
/// `structurally_assignable` (e.g. `A.f: B`, `B.g: A`), so the depth bounds that
/// recursion against cyclic shapes. Kept small: real structural-satisfaction
/// shapes match within a hop or two; a deeper chase is almost certainly a cycle.
const MAX_STRUCTURAL_DEPTH: u8 = 4;

/// Depth-carrying core of [`is_assignable_to_typed_with`]. `depth` counts how
/// many times the structural arm has re-entered for member-type comparison and
/// bounds that recursion; all other arms pass it through unchanged.
#[allow(clippy::too_many_arguments)]
fn assignable_inner(
    source: TypeId,
    target: TypeId,
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    members: &MembersIndex,
    symbol_types: &SymbolTypeMap,
    prims: &[(&str, PrimKind)],
    depth: u8,
) -> SubtypeResult {
    if source == target {
        return SubtypeResult::Yes;
    }

    let src_ty = arena.get(source).clone();
    let tgt_ty = arena.get(target).clone();

    // Top types — `any` / `unknown` accept every source. Recognized via
    // Class(qname) until the arena gains dedicated Any/Unknown primitives
    // (the canonical Type::Unknown marks "engine bailed", not "is the TS
    // top type").
    if let Type::Class(qname) = &tgt_ty {
        if matches!(qname.as_str(), "any" | "unknown") {
            return SubtypeResult::Yes;
        }
    }

    // Bottom — `never` is vacuously assignable to everything.
    if matches!(src_ty, Type::Primitive(PrimKind::Never)) {
        return SubtypeResult::Yes;
    }
    if let Type::Class(qname) = &src_ty {
        if qname == "never" {
            return SubtypeResult::Yes;
        }
    }

    // Optional target: peel one layer. `T` is assignable to `T | undefined`.
    if let Type::Optional(inner) = tgt_ty {
        return assignable_inner(
            source,
            inner,
            arena,
            lookup,
            members,
            symbol_types,
            prims,
            depth,
        );
    }

    // Union source: every branch must be assignable; one Unknown taints the
    // result to Unknown so we don't commit to Yes on incomplete evidence.
    if let Type::Union(branches) = &src_ty {
        let mut any_unknown = false;
        for b in branches {
            match assignable_inner(
                *b,
                target,
                arena,
                lookup,
                members,
                symbol_types,
                prims,
                depth,
            ) {
                SubtypeResult::Yes => continue,
                SubtypeResult::No => return SubtypeResult::No,
                SubtypeResult::Unknown => any_unknown = true,
            }
        }
        return if any_unknown {
            SubtypeResult::Unknown
        } else {
            SubtypeResult::Yes
        };
    }

    // Union target: any branch suffices. Unknown branches propagate only
    // when no Yes wins outright.
    if let Type::Union(branches) = &tgt_ty {
        let mut any_unknown = false;
        for b in branches {
            match assignable_inner(
                source,
                *b,
                arena,
                lookup,
                members,
                symbol_types,
                prims,
                depth,
            ) {
                SubtypeResult::Yes => return SubtypeResult::Yes,
                SubtypeResult::No => continue,
                SubtypeResult::Unknown => any_unknown = true,
            }
        }
        return if any_unknown {
            SubtypeResult::Unknown
        } else {
            SubtypeResult::No
        };
    }

    // Primitive identity / disjointness, bridging structural `Primitive` and
    // primitive-named `Class`: two names that both denote primitives are
    // assignable only when they denote the same kind. A primitive vs a
    // non-primitive falls through (never reject on a name not known to be a
    // primitive).
    if let (Some(a), Some(b)) = (prim_kind_of(&src_ty, prims), prim_kind_of(&tgt_ty, prims)) {
        return if a == b {
            SubtypeResult::Yes
        } else {
            SubtypeResult::No
        };
    }

    // Class → Class via inheritance walk over `parent_class_qname`.
    if let (Type::Class(src_q), Type::Class(tgt_q)) = (&src_ty, &tgt_ty) {
        let mut ancestor = src_q.clone();
        for _ in 0..MAX_INHERITANCE_HOPS {
            let Some(parent) = lookup.parent_class_qname(&ancestor) else {
                break;
            };
            if parent == tgt_q {
                return SubtypeResult::Yes;
            }
            if parent == ancestor {
                break;
            }
            ancestor = parent.to_string();
        }
        // The nominal walk found no inheritance link. Try the structural arm:
        // a source that carries every direct member of the target with a
        // matching name, matching kind, and an assignable member type satisfies
        // the target's shape (TS structural typing, Go implicit interface
        // satisfaction). The structural check is itself conservative — it only
        // ever yields Yes, never No, and stays Unknown when either member set
        // can't be fully enumerated OR a matched member lacks recorded type
        // data. We don't know the full supertype lattice yet (interfaces,
        // multi-inheritance) so a structural miss stays Unknown rather than No.
        return structurally_assignable(
            source,
            target,
            arena,
            lookup,
            members,
            symbol_types,
            prims,
            depth,
        );
    }

    SubtypeResult::Unknown
}

/// Maximum direct members compared on either side before the structural arm
/// bails to Unknown. A shape this wide is almost certainly a god-object whose
/// member set the extractor may not fully capture; treating it as
/// non-enumerable keeps the arm from declaring a false Yes on partial data.
const MAX_STRUCTURAL_MEMBERS: usize = 256;

/// Decide structural assignability between two shape-bearing types.
///
/// `source` is structurally assignable to `target` when, for every direct
/// member of `target`, `source` carries a member with the same name, the same
/// kind, AND a member type that is assignable in the correct variance:
/// - methods — source return covariant (assignable to the target's return) and
///   parameters contravariant (each target parameter assignable to the source's
///   at the same position);
/// - fields — source declared type covariant (assignable to the target's).
///
/// "Direct member" means `direct_of ∪ extensions_of` for each side — extension
/// members (C# extension methods, Rust `impl Trait for T`) are real members of
/// the shape. The supertype graph is NOT walked here: the nominal arm already
/// climbed the inheritance chain. Member-type comparison re-enters the typed
/// assignability check, bounded by `MAX_STRUCTURAL_DEPTH` against cyclic shapes.
///
/// Returns:
/// - `Yes` — every target member is present on the source with a matching kind
///   and a recorded, definitely-assignable member type.
/// - `Unknown` — either member set is empty (type has no recorded members, or is
///   an external whose members aren't hydrated), exceeds `MAX_STRUCTURAL_MEMBERS`
///   (treated as non-enumerable), the depth budget is exhausted, a matched
///   member lacks recorded `SymbolTypeData`, or a matched member's type is not
///   definitely assignable. NEVER `No`.
///
/// The arm never returns `No`: a missing or type-incompatible member means
/// "this shape doesn't match" for the positive structural path, but the
/// caller's nominal lattice is incomplete, so a structural miss is reported as
/// Unknown (undecidable) rather than a definite non-subtype. This preserves the
/// invariant that the structural arm only ever adds positive results.
///
/// Conservatism: matching a target member by `(name, kind)` alone — without
/// comparing the matched members' parameter / return types — would declare
/// `Writer{ Write(s: string) }` satisfied by `S{ Write(n: number) }`. That is a
/// false Yes. So a member whose type data is absent on either side, or whose
/// types are not definitely assignable, keeps the whole arm at Unknown.
#[allow(clippy::too_many_arguments)]
fn structurally_assignable(
    source: TypeId,
    target: TypeId,
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    members: &MembersIndex,
    symbol_types: &SymbolTypeMap,
    prims: &[(&str, PrimKind)],
    depth: u8,
) -> SubtypeResult {
    // Gate to shape-bearing kinds only. A non-Class type never reaches here
    // (the caller restricts the arm to Class → Class), but assert it locally
    // so the function stays sound if a future caller widens the entry point.
    if !matches!(arena.get(source), Type::Class(_)) || !matches!(arena.get(target), Type::Class(_))
    {
        return SubtypeResult::Unknown;
    }

    // Depth budget: member-type comparison can re-enter this function (A.f: B,
    // B.g: A). Out of budget → undecidable, not a guessed Yes.
    if depth >= MAX_STRUCTURAL_DEPTH {
        return SubtypeResult::Unknown;
    }

    let tgt_members = member_infos(target, members);
    let src_members = member_infos(source, members);

    // Enumeration conservatism: an empty set on either side means the type's
    // shape is unknown to us (no recorded members, or an external whose members
    // aren't hydrated). A type with zero members is also vacuously satisfied by
    // anything, which would be a false Yes — so empty → Unknown either way.
    if tgt_members.is_empty() || src_members.is_empty() {
        return SubtypeResult::Unknown;
    }
    if tgt_members.len() > MAX_STRUCTURAL_MEMBERS || src_members.len() > MAX_STRUCTURAL_MEMBERS {
        return SubtypeResult::Unknown;
    }

    // Every target member must be present on the source with a matching name and
    // kind AND a definitely-assignable member type. Kind is compared by exact
    // string equality: within one type's member set source and target are the
    // same language, so a `field` requirement is satisfied only by a `field` and
    // a `method` only by a `method`. A near-miss shape (missing member, a field
    // where a method is required, a member with no recorded type data, or an
    // incompatible parameter / return type) leaves the arm at Unknown.
    for needed in &tgt_members {
        let Some(have) = src_members
            .iter()
            .find(|have| have.name == needed.name && have.kind == needed.kind)
        else {
            return SubtypeResult::Unknown;
        };
        if member_types_assignable(
            have,
            needed,
            arena,
            lookup,
            members,
            symbol_types,
            prims,
            depth,
        ) != SubtypeResult::Yes
        {
            return SubtypeResult::Unknown;
        }
    }
    SubtypeResult::Yes
}

/// Decide whether the source member's type is assignable to the target
/// member's, in the correct variance. Returns `Yes` only when BOTH members
/// carry the type data the comparison needs and that data is definitely
/// assignable; any missing-type-info case returns `Unknown` (the load-bearing
/// conservatism — never fall back to name+kind when type data is absent).
///
/// Variance:
/// - method-shaped member (`return_type` recorded): source return covariant
///   (`source_return` assignable to `target_return`) and parameters
///   contravariant (each target parameter assignable to the source's at the
///   same position, via `args_assignable(source_params, target_params)`).
/// - field-shaped member (`declared_type` recorded): source declared type
///   covariant (assignable to the target's).
#[allow(clippy::too_many_arguments)]
fn member_types_assignable(
    source: &SymbolInfo,
    target: &SymbolInfo,
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    members: &MembersIndex,
    symbol_types: &SymbolTypeMap,
    prims: &[(&str, PrimKind)],
    depth: u8,
) -> SubtypeResult {
    // Both members must have recorded type data. Missing on either side →
    // Unknown. This is the conservatism that keeps a (name, kind) match from
    // standing in for an actual type comparison.
    let (Some(src_data), Some(tgt_data)) = (
        SymbolView::new(source, symbol_types).type_data(),
        SymbolView::new(target, symbol_types).type_data(),
    ) else {
        return SubtypeResult::Unknown;
    };

    let next = depth + 1;

    // Method-shaped: a recorded return type on the target signals a callable
    // member. Require the source to be callable too (its own return recorded),
    // then check return covariance and parameter contravariance.
    if let Some(tgt_return) = tgt_data.return_type {
        let Some(src_return) = src_data.return_type else {
            return SubtypeResult::Unknown;
        };
        // Return covariance: source return must be assignable to target return.
        if assignable_inner(
            src_return,
            tgt_return,
            arena,
            lookup,
            members,
            symbol_types,
            prims,
            next,
        ) != SubtypeResult::Yes
        {
            return SubtypeResult::Unknown;
        }
        // Parameter contravariance: each target parameter must be assignable to
        // the source's at the same position. `args_assignable(params, args)`
        // checks each `arg` assignable to `param`, so params = source's,
        // args = target's. A length mismatch makes it false → Unknown. A param
        // typed Unknown is treated as assignable (args_assignable is lenient on
        // missing arg-position info) — acceptable because the presence of a
        // recorded return type already established both members are callable.
        if !args_assignable_inner(
            &src_data.param_types,
            &tgt_data.param_types,
            arena,
            lookup,
            members,
            symbol_types,
            prims,
            next,
        ) {
            return SubtypeResult::Unknown;
        }
        return SubtypeResult::Yes;
    }

    // Field-shaped: covariance on the declared type. Both sides must declare it.
    match (src_data.declared_type, tgt_data.declared_type) {
        (Some(src_decl), Some(tgt_decl)) => assignable_inner(
            src_decl,
            tgt_decl,
            arena,
            lookup,
            members,
            symbol_types,
            prims,
            next,
        ),
        // No comparable type data recorded for this member → Unknown.
        _ => SubtypeResult::Unknown,
    }
}

/// Collect the direct + extension members of `ty`. The returned `SymbolInfo`
/// carries `name`, `kind`, and the DB symbol `id` the member-type comparison
/// uses to recover `SymbolTypeData`. Returns an empty Vec when the type has no
/// recorded members.
fn member_infos(ty: TypeId, members: &MembersIndex) -> Vec<&SymbolInfo> {
    members
        .direct_of(ty)
        .iter()
        .chain(members.extensions_of(ty).iter())
        .collect()
}

/// True when every `arg` is assignable to the `param` at the same position
/// (lengths must match). `Unknown` is treated as assignable — conservative for
/// overload disambiguation, where rejecting on missing evidence would drop a
/// resolution. Shared by the dispatch axes and the receiver-overload pick in
/// member lookup. `prims` is the language `primitive_mapping` so nominal
/// primitive names are recognized as disjoint.
#[allow(clippy::too_many_arguments)]
pub(crate) fn args_assignable(
    params: &[TypeId],
    args: &[TypeId],
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    members: &MembersIndex,
    symbol_types: &SymbolTypeMap,
    prims: &[(&str, PrimKind)],
) -> bool {
    args_assignable_inner(params, args, arena, lookup, members, symbol_types, prims, 0)
}

/// Depth-carrying core of [`args_assignable`]. Threads the structural-recursion
/// `depth` into each position's assignability check so a parameter comparison
/// that re-enters the structural arm stays bounded.
#[allow(clippy::too_many_arguments)]
fn args_assignable_inner(
    params: &[TypeId],
    args: &[TypeId],
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    members: &MembersIndex,
    symbol_types: &SymbolTypeMap,
    prims: &[(&str, PrimKind)],
    depth: u8,
) -> bool {
    if params.len() != args.len() {
        return false;
    }
    for (param, arg) in params.iter().zip(args.iter()) {
        match assignable_inner(
            *arg,
            *param,
            arena,
            lookup,
            members,
            symbol_types,
            prims,
            depth,
        ) {
            SubtypeResult::Yes | SubtypeResult::Unknown => continue,
            SubtypeResult::No => return false,
        }
    }
    true
}

#[cfg(test)]
#[path = "subtype_tests.rs"]
mod tests;
