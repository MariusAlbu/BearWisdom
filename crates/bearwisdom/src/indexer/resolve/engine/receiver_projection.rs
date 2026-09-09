//! Receiver-only projection: source type facts and terminal yields stay exact.
use super::{apply_args, expand_receiver, head_qname, Receiver};
use crate::indexer::resolve::engine::contract::{FileContext, SymbolLookup};
use crate::type_checker::core::types::{Indirection, Type, TypeArena};
use crate::type_checker::profile::language_profile::LanguageProfile;

/// Alias expansion and reference projection may alternate. Reference projection
/// discards the previous nominal owner; no printed name decides a reference hop.
pub(crate) fn project_receiver(
    recv: Receiver,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file: Option<&FileContext>,
    profile: &LanguageProfile,
) -> Receiver {
    project_with_borrow(recv, lookup, arena, file, profile).0
}

pub(super) fn project_with_borrow(
    mut recv: Receiver,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file: Option<&FileContext>,
    profile: &LanguageProfile,
) -> (Receiver, Option<crate::type_checker::core::types::TypeId>) {
    let mut seen = [None; 32];
    let mut borrow = None;
    for depth in 0..seen.len() {
        if !lookup.accepts_type_context(arena, recv.ty) {
            return (Receiver::untyped(arena.intern(Type::Unknown)), None);
        }
        if seen[..depth].contains(&Some(recv.ty)) {
            break;
        }
        seen[depth] = Some(recv.ty);
        let Some(projected) = super::super::program_view::project_intrinsic(lookup, arena, recv.ty)
        else {
            return (Receiver::untyped(recv.ty), None);
        };
        if projected != recv.ty {
            recv = Receiver::untyped(projected);
            continue;
        }
        if let Type::Indirect {
            kind,
            mutability,
            inner,
        } = arena.get(recv.ty)
        {
            if matches!(kind, Indirection::Reference(_)) && profile.reference_member_projection {
                borrow = Some((kind, mutability));
                recv = Receiver::untyped(inner);
                continue;
            }
            return (Receiver::untyped(recv.ty), None);
        }
        let expanded = expand_receiver(recv, lookup, arena, file);
        if expanded.ty != recv.ty {
            recv = expanded;
            continue;
        }
        let projected = peel_wrapped_receiver(expanded, arena, profile.single_inner_wrappers);
        if projected.ty == expanded.ty {
            return (
                expanded,
                borrow.map(|(kind, mutability)| {
                    arena.intern(Type::Indirect {
                        kind,
                        mutability,
                        inner: expanded.ty,
                    })
                }),
            );
        }
        borrow = None;
        recv = projected;
    }
    (Receiver::untyped(arena.intern(Type::Unknown)), None)
}

/// Peel a receiver through `profile.single_inner_wrappers` — std smart-pointer
/// heads (`Box<C>` / `Rc<C>` / `Arc<C>` / `Pin<C>` / `Cow<'a, C>`) that `Deref`
/// to their single applied argument — before member lookup runs. Peeling the
/// structural `args[0]` IS that Deref hop: `Box<Thing>.touch()` becomes
/// `Thing.touch()`. Bounded so a nested wrapper (`Arc<Box<Thing>>`) peels down
/// to `Thing`. Resets the receiver's id when a peel fires: the wrapper's own
/// declaration id no longer names the peeled type, so the caller's next
/// `expand_receiver` re-derives it from the new head. A receiver whose head is
/// absent, unlisted, or not a single-argument application returns unchanged —
/// this is why a real container (`Vec`, `HashMap`) must stay OFF the list: its
/// accessors belong to the container itself, not a peeled element.
fn peel_wrapped_receiver(recv: Receiver, arena: &TypeArena, wrappers: &[&str]) -> Receiver {
    if wrappers.is_empty() {
        return recv;
    }
    const MAX_PEEL_DEPTH: usize = 4;
    let mut ty = recv.ty;
    let mut peeled = false;
    for _ in 0..MAX_PEEL_DEPTH {
        let Some(head) = head_qname(arena, ty) else {
            break;
        };
        if !wrappers.contains(&head.as_str()) {
            break;
        }
        match apply_args(arena, ty).as_slice() {
            [arg] => {
                ty = *arg;
                peeled = true;
            }
            _ => break,
        }
    }
    if peeled {
        Receiver { ty, id: None }
    } else {
        recv
    }
}

#[cfg(test)]
#[path = "receiver_projection_tests.rs"]
mod tests;
