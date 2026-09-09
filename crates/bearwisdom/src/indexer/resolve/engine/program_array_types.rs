//! Array roles bind once to selected declarations. Queries compare only IDs.
use super::*;
use crate::indexer::lexical::type_syntax::arrays::Kind;
use crate::type_checker::core::types::{Type, TypeId, TypeOperator};

#[derive(Default)]
pub(super) struct Roles(FxHashMap<Kind, Option<i64>>);

impl Roles {
    pub(super) fn bind(view: &View, inputs: &computed_keys::Sources) -> Self {
        let mut roles = Self::default();
        let interfaces: rustc_hash::FxHashSet<_> = inputs
            .iter()
            .flat_map(|(_, _, input)| &input.interfaces)
            .filter_map(|part| view.canonical.get(&part.owner).copied())
            .collect();
        for (_, source, input) in inputs {
            for &(kind, name) in &input.array_types {
                let row = view
                    .sources
                    .get(source)
                    .and_then(|s| s.globals.get(&(name, true)))
                    .copied()
                    .flatten()
                    .and_then(|row| view.canonical.get(&row).copied())
                    .filter(|row| {
                        view.info
                            .get(row)
                            .is_some_and(|info| info.generic_param_ids.len() == 1)
                            && interfaces.contains(row)
                    });
                roles
                    .0
                    .entry(kind)
                    .and_modify(|prior| {
                        if *prior != row {
                            *prior = None;
                        }
                    })
                    .or_insert(row);
            }
        }
        if roles
            .0
            .get(&Kind::Mutable)
            .copied()
            .flatten()
            .is_some_and(|row| roles.0.get(&Kind::Readonly) == Some(&Some(row)))
        {
            roles.0.clear();
        }
        roles
    }
}

#[derive(Clone, Copy)]
pub(super) struct Shape {
    pub kind: Kind,
    pub element: TypeId,
}

pub(super) fn shape(lookup: &Lookup, arena: &TypeArena, ty: TypeId) -> Option<Shape> {
    if !(lookup as &dyn SymbolLookup).accepts_type_context(arena, ty) {
        return None;
    }
    let Type::Apply { base, args } = arena.get(ty) else {
        return None;
    };
    let [element] = args.as_slice() else {
        return None;
    };
    let Type::Decl {
        symbol_id,
        context: Some(context),
        ..
    } = arena.get(base)
    else {
        return None;
    };
    if context != lookup.view.context {
        return None;
    }
    lookup.symbol_by_id(symbol_id)?;
    let owner = lookup.canonical_decl_id(symbol_id);
    let kind = [Kind::Mutable, Kind::Readonly]
        .into_iter()
        .find(|kind| lookup.view.arrays.0.get(kind) == Some(&Some(owner)))?;
    Some(Shape {
        kind,
        element: *element,
    })
}

pub(super) fn readonly(lookup: &Lookup, arena: &TypeArena, inner: TypeId) -> Option<TypeId> {
    if matches!(arena.get(inner), Type::Tuple(_)) {
        return Some(arena.intern(Type::Operator(TypeOperator::Readonly(inner))));
    }
    let Shape {
        kind: Kind::Mutable,
        element,
    } = shape(lookup, arena, inner)?
    else {
        return None;
    };
    let owner = lookup
        .view
        .arrays
        .0
        .get(&Kind::Readonly)
        .copied()
        .flatten()?;
    let base = (lookup as &dyn SymbolLookup).declaration_type(arena, owner)?;
    Some(arena.intern(Type::Apply {
        base,
        args: vec![element],
    }))
}

/// The role establishes only container compatibility. Callers must still
/// prove the complete element relation in their chosen language phase.
pub(super) fn relation(
    lookup: &Lookup,
    arena: &TypeArena,
    from: TypeId,
    to: TypeId,
) -> Option<Result<(TypeId, TypeId), ()>> {
    let source = shape(lookup, arena, from)?;
    let target = shape(lookup, arena, to)?;
    Some(
        if source.kind == Kind::Readonly && target.kind == Kind::Mutable {
            Err(())
        } else {
            Ok((source.element, target.element))
        },
    )
}

#[cfg(test)]
#[path = "program_array_types_tests.rs"]
mod tests;
