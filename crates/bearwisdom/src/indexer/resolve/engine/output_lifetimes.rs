//! Output elision reuses input identities; it never invents a region or picks a name.
use super::contract::SymbolLookup;
use crate::type_checker::core::types::{Indirection, Lifetime, Type, TypeArena, TypeId};

pub(super) fn unique(lookup: &dyn SymbolLookup, arena: &TypeArena, inputs: &[TypeId]) -> Lifetime {
    let mut selected = None;
    let mut remaining = 4096usize;
    for &input in inputs {
        match candidate(lookup, arena, input, &mut remaining) {
            Ok(Some(region)) if selected.is_none() => selected = Some(region),
            Ok(None) => {}
            _ => return Lifetime::Unknown,
        }
    }
    selected.unwrap_or(Lifetime::Unknown)
}

fn candidate(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    input: TypeId,
    remaining: &mut usize,
) -> Result<Option<Lifetime>, ()> {
    if !lookup.accepts_type_context(arena, input) {
        return Err(());
    }
    // Distinct IDs are counted WITHIN one parameter, before alias expansion.
    // A second lifetime-bearing parameter is ambiguous even with the same ID.
    // Erased alias arguments count; hidden alias-RHS regions do not.
    let mut pending = vec![input];
    let mut selected = None;
    while let Some(id) = pending.pop() {
        *remaining = remaining.checked_sub(1).ok_or(())?;
        match arena.get(id) {
            Type::Region(region) => {
                if region == Lifetime::Unknown || selected.is_some_and(|old| old != region) {
                    return Err(());
                }
                selected = Some(region);
            }
            Type::Indirect { kind, inner, .. } => {
                if let Indirection::Reference(region) = kind {
                    pending.push(arena.intern(Type::Region(region)));
                }
                pending.push(inner);
            }
            Type::Apply { base, args } => {
                let Type::Decl { symbol_id, .. } = arena.get(base) else {
                    return Err(());
                };
                if lookup
                    .symbol_by_id(lookup.canonical_decl_id(symbol_id))
                    .is_none()
                {
                    return Err(());
                }
                pending.extend(args);
            }
            Type::Tuple(items) => pending.extend(items),
            Type::Decl { symbol_id, .. } => {
                if lookup
                    .symbol_by_id(lookup.canonical_decl_id(symbol_id))
                    .is_none()
                    || super::elided_inputs::leading_lifetimes(lookup, arena, id) > 0
                {
                    return Err(());
                }
            }
            Type::Generic { .. } | Type::Primitive(_) | Type::Literal(_) => {}
            // Nested function binders, unknown/unconfigured heads and other
            // unsupported composites are not evidence for a unique free region.
            _ => return Err(()),
        }
    }
    Ok(selected)
}

pub(super) fn application(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    region: Lifetime,
    base: TypeId,
    mut args: Vec<TypeId>,
) -> TypeId {
    let count = super::elided_inputs::leading_lifetimes(lookup, arena, base);
    if count > 0
        && !args
            .first()
            .is_some_and(|&id| matches!(arena.get(id), Type::Region(_)))
    {
        let mut complete = vec![arena.intern(Type::Region(region)); count];
        complete.append(&mut args);
        args = complete;
    }
    super::elided_inputs::checked_application(lookup, arena, base, args)
}

#[cfg(test)]
#[path = "output_lifetimes_tests.rs"]
mod tests;
