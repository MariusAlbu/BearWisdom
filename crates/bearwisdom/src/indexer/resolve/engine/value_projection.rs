//! Shared source-value projection. No display-name recovery or mutable cursor.
use crate::indexer::resolve::engine::{
    bound_call,
    chain::Receiver,
    compilation::Compilation,
    contract::{generic_return, member_applicability, SymbolLookup},
    member_index::MemberNameId,
    member_selection::{self, Selection},
    module_graph::ModuleSite,
};
use crate::type_checker::core::types::{Indirection, Type, TypeArena, TypeId};

pub(in crate::indexer::resolve::engine) struct Context<'a> {
    tree: &'a Compilation,
    site: ModuleSite,
    reference_fields: bool,
    pattern_heads: std::collections::HashMap<u32, TypeId>,
}

impl<'a> Context<'a> {
    pub(in crate::indexer::resolve::engine) fn new(
        tree: &'a Compilation,
        path: &str,
        reference_fields: bool,
    ) -> Self {
        Self {
            tree,
            site: tree.module_site(path),
            reference_fields,
            pattern_heads: Default::default(),
        }
    }
    pub(in crate::indexer::resolve::engine) fn with_patterns(
        mut self,
        heads: std::collections::HashMap<u32, TypeId>,
    ) -> Self {
        self.pattern_heads = heads;
        self
    }
    pub(super) fn variant_field(
        &self,
        arena: &TypeArena,
        ty: TypeId,
        variant: MemberNameId,
        field: MemberNameId,
        byte: u32,
    ) -> Option<TypeId> {
        let expected = self.expand(arena, *self.pattern_heads.get(&byte)?)?;
        let ty = self.expand(arena, ty)?;
        // Reference binding modes require their own evidence; never silently
        // project an owned payload from a borrowed scrutinee.
        let head = |ty| match arena.get(ty) {
            Type::Apply { base, .. } => base,
            _ => ty,
        };
        let (
            Type::Decl {
                symbol_id: actual, ..
            },
            Type::Decl {
                symbol_id: expected_id,
                ..
            },
        ) = (arena.get(head(ty)), arena.get(head(expected)))
        else {
            return None;
        };
        let owner = self.tree.canonical_decl_id(actual);
        if owner != self.tree.canonical_decl_id(expected_id)
            || self.tree.symbol_by_id(owner)?.kind != "enum"
        {
            return None;
        }
        if matches!(arena.get(expected), Type::Apply { .. }) && expected != ty {
            return None;
        }
        let accessible = |id| self.tree.accessible_at_byte(id, &self.site, byte);
        if !accessible(owner) {
            return None;
        }
        let index = self.tree.member_index()?;
        let [variant] = index.candidates(owner, variant) else {
            return None;
        };
        if self.tree.symbol_by_id(*variant)?.kind != "enum_member" || !accessible(*variant) {
            return None;
        }
        let [field] = index.candidates(*variant, field) else {
            return None;
        };
        if !matches!(
            self.tree.symbol_by_id(*field)?.kind.as_str(),
            "field" | "property"
        ) || !accessible(*field)
        {
            return None;
        }
        let yielded = self.tree.field_type_id_of(*field)?;
        let bindings = bound_call::receiver_bindings(self.tree, arena, ty, Some(owner));
        let yielded = generic_return::substitute(arena, yielded, &bindings);
        (!matches!(arena.get(yielded), Type::Unknown | Type::Class(_))).then_some(yielded)
    }
    pub(super) fn expand(&self, arena: &TypeArena, ty: TypeId) -> Option<TypeId> {
        member_applicability::expand(self.tree, arena, ty)
    }
    pub(super) fn field_receiver(&self, arena: &TypeArena, ty: TypeId) -> Option<TypeId> {
        field_receiver(self.tree, arena, ty, self.reference_fields)
    }
    pub(super) fn field(
        &self,
        arena: &TypeArena,
        ty: TypeId,
        name: MemberNameId,
        byte: u32,
    ) -> Option<TypeId> {
        let ty = self.field_receiver(arena, ty)?;
        field(self.tree, arena, ty, name, &|id| {
            self.tree.accessible_at_byte(id, &self.site, byte)
        })
    }
}

fn field_receiver(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    mut ty: TypeId,
    references: bool,
) -> Option<TypeId> {
    for _ in 0..32 {
        ty = member_applicability::expand(lookup, arena, ty)?;
        match arena.get(ty) {
            Type::Indirect {
                kind: Indirection::Reference(_),
                inner,
                ..
            } if references => ty = inner,
            Type::Indirect { .. } | Type::Class(_) | Type::Unknown => return None,
            _ => return Some(ty),
        }
    }
    None
}

fn field(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    ty: TypeId,
    name: MemberNameId,
    accessible: &dyn Fn(i64) -> bool,
) -> Option<TypeId> {
    // Only an actual nominal/application head owns nominal fields. Optional,
    // callable, pointer and union heads cannot borrow a contained declaration.
    let head = match arena.get(ty) {
        Type::Apply { base, .. } => base,
        _ => ty,
    };
    let Type::Decl { symbol_id, .. } = arena.get(head) else {
        return None;
    };
    let receiver = Receiver::new(ty, lookup.canonical_decl_id(symbol_id));
    let Selection::Unique(member) = member_selection::select_typed_with_access(
        lookup,
        arena,
        receiver,
        name,
        &|kind| matches!(kind, "property" | "field"),
        accessible,
    ) else {
        return None;
    };
    let yielded = lookup.member_info(arena, ty, member)?.field_type_id?;
    let bindings = bound_call::receiver_bindings(lookup, arena, ty, receiver.id);
    let yielded = generic_return::substitute(arena, yielded, &bindings);
    let yielded = bound_call::member_yield(lookup, arena, member, ty, yielded);
    if matches!(arena.get(yielded), Type::Unknown | Type::Class(_)) {
        return None;
    }
    Some(yielded)
}

#[cfg(test)]
#[path = "value_projection_tests.rs"]
mod tests;
