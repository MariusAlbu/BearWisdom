//! Anonymous signature parameters are source-addressed IDs, never wildcard equality.
use super::contract::SymbolLookup;
use crate::type_checker::core::types::{GenericParamKind, Lifetime, Type, TypeArena, TypeId};

pub(super) fn region(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    owner: i64,
    byte: u32,
    index: usize,
) -> TypeId {
    lookup
        .canonical_type_info(lookup.canonical_decl_id(owner))
        .and_then(|info| {
            info.elided_input_params
                .iter()
                .find(|&&(site, slot, _)| (site, slot) == (byte, index))
        })
        .map(|&(_, _, param)| arena.generic_type(param))
        .unwrap_or_else(|| arena.intern(Type::Region(Lifetime::Unknown)))
}

pub(super) fn leading_lifetimes(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    base: TypeId,
) -> usize {
    if !lookup.accepts_type_context(arena, base) {
        return 0;
    }
    let Type::Decl { symbol_id, .. } = arena.get(base) else {
        return 0;
    };
    lookup
        .canonical_type_info(lookup.canonical_decl_id(symbol_id))
        .map_or(0, |info| {
            info.generic_param_ids
                .iter()
                .take_while(|&&p| arena.generic_kind(p) == GenericParamKind::Lifetime)
                .count()
        })
}

pub(super) fn application(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    owner: i64,
    byte: u32,
    base: TypeId,
    mut args: Vec<TypeId>,
) -> TypeId {
    let count = leading_lifetimes(lookup, arena, base);
    if count > 0
        && !args
            .first()
            .is_some_and(|&id| matches!(arena.get(id), Type::Region(_)))
    {
        let mut complete: Vec<_> = (0..count)
            .map(|index| region(lookup, arena, owner, byte, index))
            .collect();
        complete.append(&mut args);
        args = complete;
    }
    checked_application(lookup, arena, base, args)
}

pub(super) fn checked_application(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    base: TypeId,
    args: Vec<TypeId>,
) -> TypeId {
    if std::iter::once(&base)
        .chain(&args)
        .any(|&ty| !lookup.accepts_type_context(arena, ty))
    {
        return arena.intern(Type::Unknown);
    }
    let count = leading_lifetimes(lookup, arena, base);
    if let Type::Decl { symbol_id, .. } = arena.get(base) {
        if let Some(info) = lookup.canonical_type_info(lookup.canonical_decl_id(symbol_id)) {
            if args.len() > info.generic_param_ids.len()
                || (count > 0 && args.len() < count)
                || info.generic_param_ids.iter().zip(&args).any(|(&p, &arg)| {
                    !super::contract::generic_return::argument_kind_agrees(arena, p, arg)
                })
            {
                return arena.intern(Type::Unknown);
            }
        }
    }
    if args.is_empty() {
        base
    } else {
        arena.intern(Type::Apply { base, args })
    }
}

/// Provision requests are source recipes, not every parameter from a namesake type.
pub(super) fn sites(
    recipe: &super::module_type_inputs::Recipe,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    materialize: &impl Fn(&super::module_type_inputs::Recipe) -> TypeId,
    out: &mut Vec<(i64, u32, usize)>,
    depth: usize,
) {
    use super::module_type_inputs::Recipe;
    if depth > 64 {
        return;
    }
    let mut child = |r| sites(r, lookup, arena, materialize, out, depth + 1);
    match recipe {
        Recipe::InputRegion { owner, byte } => out.push((*owner, *byte, 0)),
        Recipe::InputApplication {
            owner,
            byte,
            base,
            args,
        } => {
            child(base);
            for arg in args {
                child(arg);
            }
            if !args
                .first()
                .is_some_and(|arg| matches!(arena.get(materialize(arg)), Type::Region(_)))
            {
                let count = leading_lifetimes(lookup, arena, materialize(base));
                out.extend((0..count).map(|index| (*owner, *byte, index)));
            }
        }
        Recipe::Apply(base, args)
        | Recipe::Function(args, base)
        | Recipe::OutputApplication { base, args }
        | Recipe::Output {
            inputs: args,
            result: base,
        } => {
            child(base);
            for arg in args {
                child(arg);
            }
        }
        Recipe::Tuple(items) | Recipe::Union(items) | Recipe::Intersection(items) => {
            for item in items {
                child(item);
            }
        }
        Recipe::Optional(inner) => child(inner),
        Recipe::Indirect { inner, region, .. } => {
            child(inner);
            if let Some(region) = region {
                child(region);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
#[path = "elided_inputs_tests.rs"]
mod tests;
