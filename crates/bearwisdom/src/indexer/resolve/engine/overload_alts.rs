// =============================================================================
// engine/overload_alts — sibling-overload yield retry. The member index hands
// back one row of a same-qname overload set, so a called hop's yield is
// provisional: when the NEXT hop's member misses on it, the walk retries the
// sibling overloads' distinct yields — the sibling that carries the member is
// the overload the call actually bound.
// =============================================================================

use crate::indexer::resolve::engine::contract::{FileContext, Symbol, SymbolLookup};
use crate::type_checker::core::types::{TypeArena, TypeId};
use crate::type_checker::profile::language_profile::LanguageProfile;

use super::chain::{
    expand_receiver, lookup_member_on, peel_wrapped_receiver, yield_through, Receiver,
};
use super::head_decl::yielded_receiver;

/// Bound on the sibling-overload yields carried between hops.
const MAX_OVERLOAD_ALT_YIELDS: usize = 4;

/// The first alternative receiver that carries `member`, with the member
/// found on it. Each alt is probed the way the picked yield was — instance
/// members (with the supertype climb) first, then the extension-method
/// fallback, so an extension declared against the alt's supertype
/// (`Returns(this IReturnValueConfiguration<T> …)`) binds too. Consulted only
/// after every lookup on the picked yield has missed, so the picked
/// overload's own surface always wins.
pub(super) fn member_on_alt_yields(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    alts: &[Receiver],
    member: &str,
    implicit_root_types: &[&str],
) -> Option<(Symbol, Receiver)> {
    for alt in alts {
        if let Some(m) = lookup_member_on(lookup, arena, *alt, member, &|_k| true) {
            return Some((m, *alt));
        }
        if let Some(m) = super::extension_method::lookup_extension_method(
            lookup,
            arena,
            *alt,
            member,
            implicit_root_types,
        ) {
            return Some((m, *alt));
        }
    }
    None
}

/// The DISTINCT yields of `member`'s same-qname overload siblings, computed
/// with the same receiver context (`mid_recv`/`mid_id`) the picked member's
/// yield used. Yields equal to the picked overload's (`picked_ty`) are
/// dropped — only genuinely alternative surfaces are worth a retry.
#[allow(clippy::too_many_arguments)]
pub(super) fn collect_overload_alt_yields(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    member: &Symbol,
    mid_recv: TypeId,
    mid_id: Option<i64>,
    picked_ty: TypeId,
    file_ctx: &FileContext,
    profile: &LanguageProfile,
) -> Vec<Receiver> {
    let mut alts: Vec<Receiver> = Vec::new();
    for sib in lookup.all_by_qualified_name(&member.qualified_name).iter() {
        if sib.id == member.id {
            continue;
        }
        let Some(y) = yield_through(lookup, arena, sib, true, mid_recv, mid_id) else {
            continue;
        };
        let r = expand_receiver(
            peel_wrapped_receiver(
                yielded_receiver(lookup, arena, y, sib.package_id),
                arena,
                profile.single_inner_wrappers,
            ),
            lookup,
            arena,
            Some(file_ctx),
        );
        if r.ty != picked_ty && !alts.iter().any(|a| a.ty == r.ty) {
            alts.push(r);
        }
        if alts.len() >= MAX_OVERLOAD_ALT_YIELDS {
            break;
        }
    }
    alts
}
