//! Source-bound inheritance substitution. Display names are never consulted.
use super::{
    bound_call::{receiver_bindings, Bindings},
    contract::generic_return::{argument_kind_agrees, substitute},
    contract::SymbolLookup,
    head_decl::head_decl_id,
};
use crate::type_checker::core::types::{TypeArena, TypeId};
use rustc_hash::FxHashSet;

pub(super) fn captured(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    receiver: TypeId,
    id: Option<i64>,
) -> bool {
    if !lookup.accepts_type_context(arena, receiver) {
        return true;
    }
    head_decl_id(arena, receiver)
        .or(id)
        .and_then(|id| lookup.canonical_type_info(lookup.canonical_decl_id(id)))
        .is_some_and(|info| info.base_type_id.is_some())
}

/// Every reachable instantiation of the declaring owner must agree. Missing,
/// cyclic or conflicting evidence is not permission to retry a spelling path.
pub(super) fn for_member(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    receiver: TypeId,
    receiver_id: Option<i64>,
    member: i64,
) -> Option<Bindings> {
    if !lookup.accepts_type_context(arena, receiver) {
        return None;
    }
    let root = lookup.canonical_decl_id(head_decl_id(arena, receiver).or(receiver_id)?);
    let seed = receiver_bindings(lookup, arena, receiver, Some(root));
    let mut walk = Walk {
        lookup,
        arena,
        member,
        remaining: 256,
        path: FxHashSet::default(),
        found: None,
    };
    walk.visit(root, &seed).ok()?;
    walk.found
}

struct Walk<'a> {
    lookup: &'a dyn SymbolLookup,
    arena: &'a TypeArena,
    member: i64,
    remaining: usize,
    path: FxHashSet<i64>,
    found: Option<Bindings>,
}

impl Walk<'_> {
    fn visit(&mut self, owner: i64, bindings: &Bindings) -> Result<(), ()> {
        if self.remaining == 0 || self.path.len() >= 64 || !self.path.insert(owner) {
            return Err(());
        }
        self.remaining -= 1;
        self.lookup.symbol_by_id(owner).ok_or(())?;
        if self
            .lookup
            .members_of_id(owner)
            .iter()
            .any(|m| m.id == self.member)
        {
            if self.found.as_ref().is_some_and(|prior| prior != bindings) {
                return Err(());
            }
            self.found = Some(bindings.clone());
        } else {
            for parent in self.lookup.parent_class_ids(owner) {
                let parent = self.lookup.canonical_decl_id(parent);
                let info = self.lookup.canonical_type_info(parent).ok_or(())?;
                let args = self.lookup.parent_class_arg_ids_of(owner, parent);
                if args.len() != info.generic_param_ids.len() {
                    return Err(());
                }
                let mut next = Bindings::default();
                for (&param, &arg) in info.generic_param_ids.iter().zip(args) {
                    let arg = substitute(self.arena, arg, bindings);
                    if !self.lookup.accepts_type_context(self.arena, arg)
                        || !argument_kind_agrees(self.arena, param, arg)
                    {
                        return Err(());
                    }
                    next.insert(param, arg);
                }
                self.visit(parent, &next)?;
            }
        }
        self.path.remove(&owner);
        Ok(())
    }
}

#[cfg(test)]
#[path = "inherited_bindings_tests.rs"]
mod tests;
