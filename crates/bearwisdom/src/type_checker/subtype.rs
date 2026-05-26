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
// PR 16.
// =============================================================================

use crate::indexer::resolve::engine::SymbolLookup;
use crate::type_checker::core::types::{PrimKind, Type, TypeArena, TypeId};

/// TypeScript primitives the conservative branch uses to detect
/// disjoint primitive pairs (e.g. `"string" extends number ? ...`).
/// Kept narrow on purpose — additions here change branch selection
/// across every conditional alias in the index.
const PRIMITIVES: &[&str] = &[
    "string", "number", "boolean", "bigint", "symbol", "null", "undefined", "void",
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
) -> SubtypeResult {
    is_assignable_to_typed_with(source, target, arena, lookup, &[])
}

/// Map a type to its primitive kind for disjointness checks. A structural
/// `Primitive` yields its kind directly; a nominal `Class` yields a kind only
/// when its qname appears in `prims` (a language's `primitive_mapping`). This
/// bridges the two ways a primitive annotation can be interned, so `string`
/// vs `number` is decided whether either side is `Primitive(..)` or `Class(..)`.
fn prim_kind_of(ty: &Type, prims: &[(&str, PrimKind)]) -> Option<PrimKind> {
    match ty {
        Type::Primitive(k) => Some(*k),
        Type::Class(q) => prims.iter().find(|(n, _)| *n == q.as_str()).map(|(_, k)| *k),
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
///   `MAX_INHERITANCE_HOPS` matching the string form).
/// - `Primitive` → `Primitive`: equal kinds → Yes, different kinds → No.
///
/// Everything else returns `Unknown`.
pub fn is_assignable_to_typed_with(
    source: TypeId,
    target: TypeId,
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    prims: &[(&str, PrimKind)],
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
        return is_assignable_to_typed_with(source, inner, arena, lookup, prims);
    }

    // Union source: every branch must be assignable; one Unknown taints the
    // result to Unknown so we don't commit to Yes on incomplete evidence.
    if let Type::Union(branches) = &src_ty {
        let mut any_unknown = false;
        for b in branches {
            match is_assignable_to_typed_with(*b, target, arena, lookup, prims) {
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
            match is_assignable_to_typed_with(source, *b, arena, lookup, prims) {
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
                return SubtypeResult::Unknown;
            };
            if parent == tgt_q {
                return SubtypeResult::Yes;
            }
            if parent == ancestor {
                break;
            }
            ancestor = parent.to_string();
        }
        // Walked the full chain without finding the target. We don't know
        // the full supertype lattice yet (interfaces, structural types,
        // multi-inheritance) so call this Unknown rather than No — matches
        // the string form's "primitive vs user-type is undecidable" stance.
        return SubtypeResult::Unknown;
    }

    SubtypeResult::Unknown
}

#[cfg(test)]
#[path = "subtype_tests.rs"]
mod tests;
