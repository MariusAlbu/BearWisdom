//! Explicit trait/Self selection shares obligations, not dot-call adjustment order.
use super::super::{contract::flow_cache::BoundCall, trait_graph::QualifiedCall};
use super::*;

pub(in super::super) fn select(
    graph: &Graph,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    call: &QualifiedCall,
    name: MemberNameId,
    actual: &[TypeId],
    explicit: &[TypeId],
) -> Result<BoundCall, ()> {
    let bound = call.obligation;
    if explicit.iter().any(|&ty| !obligations::known(arena, ty)) {
        return Err(());
    }
    let id = super::super::head_decl::head_decl_id(arena, bound.trait_type).ok_or(())?;
    if !lookup.declaration_accessible(id)
        || super::super::head_decl::head_decl_id(arena, bound.subject)
            .is_some_and(|id| !lookup.declaration_accessible(id))
    {
        return Err(());
    }
    let definition = graph.definitions.get(&id).filter(|d| d.enabled).ok_or(())?;
    let members = definition.members.get(&name).ok_or(())?;
    let [member] = members.as_slice() else {
        return Err(());
    };
    let callee = lookup.symbol_by_id(*member).ok_or(())?;
    let info = lookup.canonical_type_info(*member).ok_or(())?;
    let assumptions = graph.assumptions(call.caller.ok_or(())?, arena)?;
    if !obligations::prove(
        graph,
        lookup,
        arena,
        bound,
        &assumptions,
        &mut FxHashSet::default(),
        &mut 4096,
    )? {
        return Err(());
    }
    let mut bindings = graph
        .bindings(id, bound.subject, bound.trait_type, arena)
        .ok_or(())?;
    let receiver_arguments = usize::from(info.receiver_type_id.is_some());
    let ordinary = actual.get(receiver_arguments..).ok_or(())?;
    let patterns = info.parameter_type_ids.as_ref().ok_or(())?;
    if ordinary.len() != patterns.len() {
        return Err(());
    }
    let adjusted = if let Some(pattern) = info.receiver_type_id {
        let receiver = *actual.first().ok_or(())?;
        bindings.extend(
            receiver_match(lookup, arena, *member, pattern, receiver, &bindings)?.ok_or(())?,
        );
        Some(receiver)
    } else {
        None
    };
    let bindings = super::super::bound_call::environment_with_initial(
        lookup,
        arena,
        callee,
        bound.subject,
        super::super::head_decl::head_decl_id(arena, bound.subject),
        explicit,
        ordinary,
        adjusted,
        &bindings,
    )
    .ok_or(())?;
    if !obligations::all(
        graph,
        lookup,
        arena,
        *member,
        &bindings,
        &assumptions,
        &mut FxHashSet::default(),
        &mut 4096,
    )? {
        return Err(());
    }
    let substitute = |ty| super::super::contract::generic_return::substitute(arena, ty, &bindings);
    let parameters: Vec<_> = patterns.iter().copied().map(substitute).collect();
    for (&pattern, &actual) in parameters.iter().zip(ordinary) {
        if !matches!(
            (ReceiverPattern {
                ty: pattern,
                parameters: vec![]
            })
            .bindings(lookup, arena, actual),
            Ok(Some(_))
        ) {
            return Err(());
        }
    }
    Ok(BoundCall {
        declaration: *member,
        return_type: info.return_type_id.map(substitute),
        parameters,
        receiver_arguments,
    })
}

#[cfg(test)]
#[path = "trait_qualified_call_tests.rs"]
mod tests;
