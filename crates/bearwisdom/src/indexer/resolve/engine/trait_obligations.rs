//! Bounded proof of source trait obligations. Unknown never establishes applicability.
use super::super::trait_graph::{substitute, Obligation};
use super::*;

pub(super) fn prove(
    graph: &Graph,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    wanted: Obligation,
    assumptions: &[Obligation],
    active: &mut FxHashSet<Obligation>,
    budget: &mut usize,
) -> Result<bool, ()> {
    *budget = budget.checked_sub(1).ok_or(())?;
    if !lookup.accepts_type_context(arena, wanted.subject)
        || !lookup.accepts_type_context(arena, wanted.trait_type)
    {
        return Err(());
    }
    if !active.insert(wanted) {
        return Err(());
    }
    let result = prove_inner(graph, lookup, arena, wanted, assumptions, active, budget);
    active.remove(&wanted);
    result
}

fn prove_inner(
    graph: &Graph,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    wanted: Obligation,
    assumptions: &[Obligation],
    active: &mut FxHashSet<Obligation>,
    budget: &mut usize,
) -> Result<bool, ()> {
    if !known(arena, wanted.subject) || !known(arena, wanted.trait_type) {
        return Err(());
    }
    let id = super::super::head_decl::head_decl_id(arena, wanted.trait_type).ok_or(())?;
    if !graph.definitions.get(&id).is_some_and(|d| d.enabled) {
        return Err(());
    }
    if assumptions.contains(&wanted) {
        return Ok(true);
    }
    let mut found = false;
    let mut unknown = false;
    for implementation in graph
        .implementations
        .get(&Some(id))
        .into_iter()
        .flatten()
        .chain(graph.implementations.get(&None).into_iter().flatten())
    {
        let pattern = ReceiverPattern {
            ty: arena.intern(Type::Tuple(vec![
                implementation.receiver.ty,
                implementation.trait_type,
            ])),
            parameters: implementation.receiver.parameters.clone(),
        };
        let actual = arena.intern(Type::Tuple(vec![wanted.subject, wanted.trait_type]));
        let bindings = match pattern.bindings(lookup, arena, actual) {
            Ok(Some(bindings)) => bindings,
            Ok(None) => continue,
            Err(()) => {
                unknown = true;
                continue;
            }
        };
        if !implementation.enabled {
            unknown = true;
            continue;
        }
        if implementation.negative {
            return Ok(false);
        }
        match all(
            graph,
            lookup,
            arena,
            implementation.declaration,
            &bindings,
            assumptions,
            active,
            budget,
        ) {
            Ok(true) if found => return Err(()),
            Ok(true) => found = true,
            Ok(false) => {}
            Err(()) => unknown = true,
        }
    }
    if unknown {
        Err(())
    } else {
        Ok(found)
    }
}

pub(super) fn known(arena: &TypeArena, ty: TypeId) -> bool {
    let mut pending = vec![ty];
    let mut budget = 4096usize;
    while let Some(ty) = pending.pop() {
        let Some(next) = budget.checked_sub(1) else {
            return false;
        };
        budget = next;
        match arena.get(ty) {
            Type::Unknown
            | Type::Class(_)
            | Type::Operator(_)
            | Type::Region(Lifetime::Unknown) => return false,
            Type::Indirect {
                kind: Indirection::Reference(Lifetime::Unknown),
                ..
            } => return false,
            Type::Apply { base, args } => {
                pending.push(base);
                pending.extend(args);
            }
            Type::Tuple(items) | Type::Union(items) | Type::Intersection(items) => {
                pending.extend(items)
            }
            Type::Function { params, return_ } => {
                pending.extend(params);
                pending.push(return_);
            }
            Type::Indirect { inner, .. }
            | Type::Optional(inner)
            | Type::AsyncWrapper(inner)
            | Type::Iterator(inner)
            | Type::Constructor(inner) => pending.push(inner),
            _ => {}
        }
    }
    true
}

pub(super) fn all(
    graph: &Graph,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    owner: i64,
    bindings: &super::super::bound_call::Bindings,
    assumptions: &[Obligation],
    active: &mut FxHashSet<Obligation>,
    budget: &mut usize,
) -> Result<bool, ()> {
    let mut unknown = false;
    for bound in graph.bounds.get(&owner).into_iter().flatten() {
        match prove(
            graph,
            lookup,
            arena,
            substitute(*bound, arena, bindings),
            assumptions,
            active,
            budget,
        ) {
            Ok(false) => return Ok(false),
            Ok(true) => {}
            Err(()) => unknown = true,
        }
    }
    if unknown {
        Err(())
    } else {
        Ok(true)
    }
}

#[cfg(test)]
#[path = "trait_obligations_tests.rs"]
mod tests;
