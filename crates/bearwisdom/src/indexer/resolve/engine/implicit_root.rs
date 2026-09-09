// =============================================================================
// engine/implicit_root — closes the member walk at the language's implicit root
// type: the base every declaration inherits without writing it. A receiver whose
// source carries NO base list still exposes the root's members, so a walk that
// exhausts the declared supertypes probes the root's own declaration before the
// hop is reported as a member miss.
// =============================================================================

use crate::indexer::resolve::engine::contract::{Symbol, SymbolLookup};
use crate::type_checker::core::types::TypeArena;
use crate::type_checker::profile::language_profile::LanguageProfile;

use super::chain::{lookup_member, lookup_member_by_id, lookup_member_on, Receiver};

/// Resolve `member` on `recv` — the declared member walk, closed at the
/// language's implicit root types (`profile.implicit_root_types`).
///
/// The declared walk runs FIRST and always wins: own members, the id-keyed and
/// qname supertype climbs, and the composite / mapped / index-signature
/// fallbacks all resolve before the root is consulted, so a declared
/// supertype's member is never shadowed by the root's. The root probe runs once
/// per exhausted walk, never per climb level, and the root never enters the
/// climb's frontier. A profile with no root names leaves the walk unchanged.
///
/// The probe is gated on a BOUND receiver declaration (`recv.id`): only a
/// receiver the walk actually typed — whose declared climb genuinely exhausted
/// — closes at the root. An untyped receiver keeps its miss, so the ref stays
/// visible as evidence of the upstream capture gap instead of binding the
/// root's member on no type information.
pub(super) fn walk_member(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    recv: Receiver,
    member: &str,
    profile: &LanguageProfile,
) -> Result<Option<Symbol>, super::member_selection::Selection> {
    // Keep declared access/ambiguity failures distinct from a miss across every
    // outer retry (implicit roots, deref, extensions and alternative yields).
    use super::member_selection::{self, Selection};
    if let Some(owner) = recv.id {
        if !lookup.declaration_accessible(owner) {
            return Err(Selection::Inaccessible);
        }
        if let Some(name) = lookup.member_index().and_then(|index| index.name(member)) {
            match member_selection::select_typed(lookup, arena, recv, name, &|_| true) {
                Selection::Unique(id) => return Ok(lookup.symbol_by_id(id).cloned()),
                Selection::Missing => {}
                denied => return Err(denied),
            }
        }
    }
    Ok(
        lookup_member_on(lookup, arena, recv, member, &|_kind| true).or_else(|| {
            if recv.id.is_none() {
                return None;
            }
            member_on_implicit_root(lookup, member, profile.implicit_root_types)
        }),
    )
}

/// Resolve `member` on the declaration one of `roots` names, climbing that
/// declaration's own supertypes. Each root name is tried in order and the first
/// declaration carrying `member` wins. `None` when `roots` is empty, no root
/// name is indexed as a type, or none of them declares `member` — the walk's
/// miss stands, with the cause the caller already attributes.
pub(super) fn member_on_implicit_root(
    lookup: &dyn SymbolLookup,
    member: &str,
    roots: &[&str],
) -> Option<Symbol> {
    let accept = |_kind: &str| true;
    for root in roots {
        for cand in root_declarations(lookup, root) {
            // The identity path first — the root's members and its own base
            // climb key on the declaration id; the qname path covers a root
            // whose members are indexed by qualified name only.
            let hit = lookup_member_by_id(lookup, cand.id, member, &accept)
                .or_else(|| lookup_member(lookup, &cand.qualified_name, member, &accept));
            if let Some(m) = hit {
                crate::tracef!(
                    "  ROOT '{}' on {} -> {}",
                    member,
                    cand.qualified_name,
                    m.qualified_name,
                );
                return Some(m);
            }
        }
    }
    None
}

/// The indexed type declarations named `root`, most-likely-root first: an
/// EXTERNAL row before a project one, a QUALIFIED name before a bare one. The
/// implicit root is a library declaration, so a project type that happens to
/// share its simple name must not win the probe. `types_by_name` already
/// restricts the set to type-like kinds.
fn root_declarations<'a>(lookup: &'a dyn SymbolLookup, root: &str) -> Vec<&'a Symbol> {
    let mut cands: Vec<&Symbol> = lookup.types_by_name(root).into_iter().collect();
    cands.sort_by_key(|s| {
        (
            !lookup.is_external_file(&s.file_path),
            !s.qualified_name.contains('.'),
        )
    });
    cands
}

#[cfg(test)]
#[path = "implicit_root_tests.rs"]
mod tests;
