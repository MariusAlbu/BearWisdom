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

#[cfg(test)]
#[path = "subtype_tests.rs"]
mod tests;
