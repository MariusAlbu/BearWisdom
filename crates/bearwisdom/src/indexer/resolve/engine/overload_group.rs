// =============================================================================
// engine/overload_group — one owner's same-name callable declarations are ONE
// member, not a field of competing candidates.
//
// Member selection counts the declarations a level contributes and reports
// more than one as ambiguous. Two shapes hide behind that count: a name
// several OWNERS declare, where nothing at this level says which the receiver
// means, and a single owner's OVERLOAD SET, where every row is the same member
// of the same type under a different signature. Only the first is ambiguous.
// The walk's contract for the second is already written into the member step —
// one row of the set is handed back and the hop's yield stays provisional, the
// sibling yields being retried when the next hop misses on it. This module
// decides which shape a level holds and names the row representing an
// overload set.
// =============================================================================

use rustc_hash::FxHashSet;

use super::chain::is_callable;
use super::contract::SymbolLookup;

/// What a level holding several rows of ONE owner's member means to the caller
/// asking for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Overloads {
    /// The set stands for the member: its representative row answers the hop.
    Represent,
    /// The caller selects over the set itself, from the call's argument types.
    /// Reporting the set keeps that stronger evidence from being pre-empted.
    Report,
}

/// The declaration representing `candidates` when they are one owner's
/// overload set: the lowest id, so the pick never depends on the order the
/// member index handed the rows over.
///
/// `Some` only when `one_owner` holds and EVERY candidate is callable. A set
/// spanning owners names no single member, and a set mixing a callable with a
/// field or property is a homonym only the hop's kind context can separate;
/// both stay ambiguous.
pub(super) fn select(
    lookup: &dyn SymbolLookup,
    candidates: &FxHashSet<i64>,
    one_owner: bool,
) -> Option<i64> {
    if !one_owner {
        return None;
    }
    let mut representative: Option<i64> = None;
    for &id in candidates {
        if !is_callable(lookup.symbol_by_id(id)?.kind.as_str()) {
            return None;
        }
        representative = Some(representative.map_or(id, |best| best.min(id)));
    }
    representative
}

#[cfg(test)]
#[path = "overload_group_tests.rs"]
mod tests;
