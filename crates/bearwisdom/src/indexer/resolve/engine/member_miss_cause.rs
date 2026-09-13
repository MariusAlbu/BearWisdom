// =============================================================================
// engine/member_miss_cause — why a member lookup missed on a walked receiver
//
// The member walk reaches a hop, looks the segment's name up on the receiver in
// hand, and finds nothing. This module names that miss from state already held:
// the receiver's declaration when the walk bound one, and the shape of the
// head's own name in the index when it did not. A receiver the walk typed by
// NAME but never bound to a declaration is a reachability or a supply fact about
// that head — a cause, not an absence of diagnosis.
// =============================================================================

use crate::type_checker::core::types::TypeArena;
use crate::types::AliasTargetIds;

use super::cause::{Cause, CauseKind};
use super::chain::{head_qname, Receiver};
use super::contract::SymbolLookup;
use super::support::index_qname_leaf;

/// The cause of a member-lookup miss on `recv`.
///
/// A receiver bound to a declaration blames that declaration: an external type
/// carrying no materialized members is an externals-pipeline gap, anything else
/// genuinely lacks the member. A receiver typed only by name blames its head —
/// a capture-only alias arm that can never yield a member set, a declaration the
/// index holds but no rung reached from this file, or a name the index holds no
/// type for at all.
///
/// `None` only when the state the cause would name is itself absent: a receiver
/// id no symbol row answers to, or a type with no nominal head.
pub(super) fn classify(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    recv: Receiver,
) -> Option<Cause> {
    if let Some(id) = recv.id {
        let sym = lookup.symbol_by_id(id)?;
        let has_members = !lookup.members_of_id(id).is_empty()
            || !lookup.members_of(&sym.qualified_name).is_empty();
        return Some(if sym.file_path.starts_with("ext:") && !has_members {
            Cause::new(Some(id), CauseKind::ExternalUnmaterialized)
        } else {
            Cause::new(Some(id), CauseKind::MemberMissing)
        });
    }
    let head = head_qname(arena, recv.ty)?;
    match lookup.alias_target(&head) {
        Some(AliasTargetIds::Union(_))
        | Some(AliasTargetIds::Intersection(_))
        | Some(AliasTargetIds::Keyof(_))
        | Some(AliasTargetIds::Other) => Some(Cause::new(
            lookup.by_qualified_name(&head).map(|s| s.id),
            CauseKind::AliasOpaque,
        )),
        _ => Some(unbound_head_cause(lookup, &head)),
    }
}

/// The cause for a head the walk typed by name and never bound to a declaration:
/// the index holds at least one type declaration of that simple name — the
/// declaration exists and no rung reached it from here — or it holds none, and
/// the type's own supply is missing. Blames the declaration only when it is
/// unique; an ambiguous name has no single symbol to blame. The same floor the
/// bare-name classifier applies, asked about a receiver head instead of a root.
fn unbound_head_cause(lookup: &dyn SymbolLookup, head: &str) -> Cause {
    let declarations = lookup.types_by_name(index_qname_leaf(head));
    if declarations.is_empty() {
        return Cause::new(None, CauseKind::NameUnknown);
    }
    let blame = if declarations.len() == 1 {
        declarations.first().map(|s| s.id)
    } else {
        None
    };
    Cause::new(blame, CauseKind::DefinedUnimported)
}

#[cfg(test)]
#[path = "member_miss_cause_tests.rs"]
mod tests;
