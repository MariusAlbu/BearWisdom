//! Nominal operations share the lookup's selected environment and ID ownership.
use super::SymbolLookup;
use crate::type_checker::core::types::{TypeArena, TypeId};

impl dyn SymbolLookup + '_ {
    pub(crate) fn projected_member_info(
        &self,
        arena: &TypeArena,
        receiver: TypeId,
        member: i64,
    ) -> Option<&super::TypeInfo> {
        if !self.accepts_type_context(arena, receiver) {
            return None;
        }
        let owner = super::super::head_decl::head_decl_id(arena, receiver)?;
        self.receiver_member_info(self.canonical_decl_id(owner), member)
    }
    pub(crate) fn member_info(
        &self,
        arena: &TypeArena,
        receiver: TypeId,
        member: i64,
    ) -> Option<&super::TypeInfo> {
        if !self.accepts_type_context(arena, receiver) {
            return None;
        }
        self.projected_member_info(arena, receiver, member)
            .or_else(|| self.canonical_type_info(member))
    }
    /// Outer None means no projection. A missing projected value is authoritative.
    pub(crate) fn projected_member_yield(
        &self,
        arena: &TypeArena,
        receiver: TypeId,
        member: i64,
        call: bool,
    ) -> Option<Option<TypeId>> {
        if let Some(value) = self.object_member_type(receiver, member) {
            return Some(
                value
                    .filter(|&ty| self.accepts_type_context(arena, ty))
                    .and_then(|ty| {
                        if call {
                            match arena.get(ty) {
                                crate::type_checker::core::types::Type::Callable(c) => {
                                    Some(c.result)
                                }
                                crate::type_checker::core::types::Type::Function {
                                    return_,
                                    ..
                                } => Some(return_),
                                _ => None,
                            }
                        } else {
                            Some(ty)
                        }
                    }),
            );
        }
        let info = self.projected_member_info(arena, receiver, member)?;
        let value = if call {
            info.return_type_id.or(info.field_type_id)
        } else {
            info.field_type_id
        };
        Some(
            value
                .filter(|&ty| self.accepts_type_context(arena, ty))
                .map(|ty| {
                    let ty = if call {
                        match arena.get(ty) {
                            crate::type_checker::core::types::Type::Function {
                                return_, ..
                            } => return_,
                            _ => ty,
                        }
                    } else {
                        ty
                    };
                    let bindings =
                        super::super::bound_call::receiver_bindings(self, arena, receiver, None);
                    super::generic_return::substitute(arena, ty, &bindings)
                }),
        )
    }
    pub(crate) fn accepts_type_context(&self, arena: &TypeArena, ty: TypeId) -> bool {
        arena.accepts_nominal_context(ty, self.nominal_context())
    }
    /// A view supplies the canonical owner by ID. Display payloads
    /// do not select a declaration; they are copied only on first interning.
    pub(crate) fn declaration_type(&self, arena: &TypeArena, id: i64) -> Option<TypeId> {
        let id = self.canonical_decl_id(id);
        let symbol = self.symbol_by_id(id)?;
        Some(match self.nominal_context() {
            Some(context) => arena.decl_in(context, &symbol.qualified_name, id),
            None => arena.decl(&symbol.qualified_name, id),
        })
    }
}

#[cfg(test)]
#[path = "lookup_nominal_tests.rs"]
mod tests;
