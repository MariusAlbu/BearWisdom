//! Alias steps with captured declaration and generic-parameter identities.
use super::*;

pub(super) fn step(
    ty: TypeId,
    receiver: Option<i64>,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
) -> Option<TypeId> {
    if !lookup.accepts_type_context(arena, ty) {
        return Some(arena.intern(Type::Unknown));
    }
    let id = super::super::head_decl::head_decl_id(arena, ty).or(receiver)?;
    let template = lookup.canonical_type_info(id)?.lexical_alias.as_ref()?;
    Some(
        template
            .instantiate(arena, &apply_args(arena, ty))
            .unwrap_or_else(|| arena.intern(Type::Unknown)),
    )
}

/// Existing structural application construction, shared by legacy alias arms.
pub(super) fn application_target(arena: &TypeArena, root: TypeId, args: &[TypeId]) -> TypeId {
    if args.is_empty() {
        root
    } else {
        arena.intern(Type::Apply {
            base: root,
            args: args.to_vec(),
        })
    }
}

#[cfg(test)]
#[path = "alias_bound_tests.rs"]
mod tests;
