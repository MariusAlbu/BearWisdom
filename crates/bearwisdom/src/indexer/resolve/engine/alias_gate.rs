// =============================================================================
// engine/alias_gate — contested-head gate for name-keyed alias expansion
//
// A type alias's target is looked up by NAME as a fallback when no declaration
// id is in hand. That name key can collide with a nominal type: declaration
// merging puts a class / namespace / interface under the same qname as an
// alias, and the simple-name key registered for a qualified alias can collide
// with an unrelated type's bare name. Following the alias there swaps a
// receiver that already names a member-bearing nominal type for the alias's
// target, and every member on it misses. The gate refuses the name-keyed
// target on such a collision; the id-keyed target path is never gated —
// identity always wins.
// =============================================================================

use crate::types::AliasTargetIds;

use super::contract::{is_type_like_kind, SymbolLookup};

/// `true` when some declaration sharing `head`'s exact qualified name is a
/// non-alias type-like kind — the head names a nominal type, so a name-keyed
/// alias lookup must not follow a same-named alias's target through it. Scans
/// ALL same-qname declarations, so the answer is independent of which one won
/// the first-winner `by_qualified_name` slot.
pub(super) fn head_names_nominal_type(lookup: &dyn SymbolLookup, head: &str) -> bool {
    lookup
        .all_by_qualified_name(head)
        .iter()
        .any(|s| is_type_like_kind(&s.kind) && s.kind != "type_alias")
}

/// The name-keyed alias target for `head`, refused when the head is contested
/// by a nominal type declaration. A dot-free head additionally probes the
/// simple-name type index: the simple-name key registered for a qualified
/// alias can collide with a type whose own qname is qualified, which the exact
/// qname scan cannot see. Returns the target only for an uncontested head.
pub(super) fn uncontested_alias_target<'l>(
    lookup: &'l dyn SymbolLookup,
    head: &str,
) -> Option<&'l AliasTargetIds> {
    if head_names_nominal_type(lookup, head) {
        return None;
    }
    if !head.contains('.')
        && lookup
            .types_by_name(head)
            .iter()
            .any(|s| is_type_like_kind(&s.kind) && s.kind != "type_alias")
    {
        return None;
    }
    lookup.alias_target(head)
}
