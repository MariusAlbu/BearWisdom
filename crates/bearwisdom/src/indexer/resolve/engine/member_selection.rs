//! ID-only nominal member traversal. Candidate order never selects a declaration.
use super::{contract::SymbolLookup, member_index::MemberNameId, overload_group::Overloads};
use rustc_hash::FxHashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Selection {
    Missing,
    Unique(i64),
    Ambiguous,
    Incomplete,
    Inaccessible,
}

pub(super) fn select(
    lookup: &dyn SymbolLookup,
    owner: i64,
    name: MemberNameId,
    accept: &dyn Fn(&str) -> bool,
) -> Selection {
    select_with_receiver(
        lookup,
        owner,
        name,
        accept,
        None,
        &|id| lookup.declaration_accessible(id),
        Overloads::Represent,
    )
}

pub(super) fn select_typed(
    lookup: &dyn SymbolLookup,
    arena: &crate::type_checker::core::types::TypeArena,
    receiver: super::chain::Receiver,
    name: MemberNameId,
    accept: &dyn Fn(&str) -> bool,
) -> Selection {
    select_typed_with_access(lookup, arena, receiver, name, accept, &|id| {
        lookup.declaration_accessible(id)
    })
}

/// Selection that reports an overload set instead of representing it by one
/// row — for a caller holding the call's arguments, whose own selection over
/// the set is stronger evidence than the group's representative.
pub(super) fn select_typed_exact(
    lookup: &dyn SymbolLookup,
    arena: &crate::type_checker::core::types::TypeArena,
    receiver: super::chain::Receiver,
    name: MemberNameId,
    accept: &dyn Fn(&str) -> bool,
) -> Selection {
    typed(
        lookup,
        arena,
        receiver,
        name,
        accept,
        &|id| lookup.declaration_accessible(id),
        Overloads::Report,
    )
}

pub(super) fn select_typed_with_access(
    lookup: &dyn SymbolLookup,
    arena: &crate::type_checker::core::types::TypeArena,
    receiver: super::chain::Receiver,
    name: MemberNameId,
    accept: &dyn Fn(&str) -> bool,
    accessible: &dyn Fn(i64) -> bool,
) -> Selection {
    typed(
        lookup,
        arena,
        receiver,
        name,
        accept,
        accessible,
        Overloads::Represent,
    )
}

fn typed(
    lookup: &dyn SymbolLookup,
    arena: &crate::type_checker::core::types::TypeArena,
    receiver: super::chain::Receiver,
    name: MemberNameId,
    accept: &dyn Fn(&str) -> bool,
    accessible: &dyn Fn(i64) -> bool,
    overloads: Overloads,
) -> Selection {
    if !lookup.accepts_type_context(arena, receiver.ty) {
        return Selection::Incomplete;
    }
    let Some(owner) = receiver.id else {
        return Selection::Incomplete;
    };
    select_with_receiver(
        lookup,
        owner,
        name,
        accept,
        Some((arena, receiver.ty)),
        accessible,
        overloads,
    )
}

fn select_with_receiver(
    lookup: &dyn SymbolLookup,
    owner: i64,
    name: MemberNameId,
    accept: &dyn Fn(&str) -> bool,
    receiver: Option<(
        &crate::type_checker::core::types::TypeArena,
        crate::type_checker::core::types::TypeId,
    )>,
    accessible: &dyn Fn(i64) -> bool,
    overloads: Overloads,
) -> Selection {
    let Some(index) = lookup.member_index() else {
        return Selection::Incomplete;
    };
    let mut visited = FxHashSet::default();
    let mut frontier = vec![owner];
    let mut remaining = 65_536usize;
    for _ in 0..super::chain::MAX_SUPERTYPE_DEPTH {
        let mut next = Vec::new();
        let mut candidates = FxHashSet::default();
        let mut declared = false;
        // The owner every candidate so far was declared on, and whether that
        // has held. A level can draw from several owners at once (a receiver
        // reaching two supertypes), and only a level drawn from ONE owner can
        // be an overload set.
        let mut declaring_owner: Option<i64> = None;
        let mut one_owner = true;
        for owner in frontier {
            let owner = lookup.canonical_decl_id(owner);
            if !accessible(owner) {
                return Selection::Inaccessible;
            }
            if !visited.insert(owner) {
                continue;
            }
            let Some(budget) = remaining.checked_sub(1) else {
                return Selection::Incomplete;
            };
            remaining = budget;
            let members = index.candidates(owner, name);
            if members.is_empty() && index.declared(owner, name) {
                return Selection::Incomplete;
            }
            for &member in members {
                let Some(budget) = remaining.checked_sub(1) else {
                    return Selection::Incomplete;
                };
                remaining = budget;
                let Some(member) = lookup.symbol_by_id(member) else {
                    return Selection::Incomplete;
                };
                if let Some(pattern) = lookup.member_pattern(member.id) {
                    let Some((arena, ty)) = receiver else {
                        return Selection::Incomplete;
                    };
                    match pattern.bindings(lookup, arena, ty) {
                        Ok(Some(_)) => {}
                        Ok(None) => continue,
                        Err(()) => return Selection::Incomplete,
                    }
                }
                // Only a member of an admitted kind declares the name at this
                // level: a kind the hop cannot address (a property under a
                // call in a language with separate member namespaces) neither
                // competes nor hides an admitted member further up the climb.
                if accept(&member.kind) {
                    declared = true;
                    if !accessible(member.id) {
                        return Selection::Inaccessible;
                    }
                    match declaring_owner {
                        None => declaring_owner = Some(owner),
                        Some(seen) => one_owner &= seen == owner,
                    }
                    candidates.insert(lookup.canonical_decl_id(member.id));
                }
            }
            next.extend(lookup.parent_class_ids(owner));
        }
        if declared {
            return match candidates.len() {
                0 => Selection::Missing,
                1 => Selection::Unique(*candidates.iter().next().unwrap()),
                _ => match overloads {
                    Overloads::Report => Selection::Ambiguous,
                    Overloads::Represent => {
                        super::overload_group::select(lookup, &candidates, one_owner)
                            .map_or(Selection::Ambiguous, Selection::Unique)
                    }
                },
            };
        }
        if next.is_empty() {
            return Selection::Missing;
        }
        frontier = next;
    }
    Selection::Incomplete
}

#[cfg(test)]
#[path = "member_selection_tests.rs"]
mod tests;
