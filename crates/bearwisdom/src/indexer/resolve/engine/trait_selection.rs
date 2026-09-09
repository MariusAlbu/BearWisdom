//! Ordered, source-attested method selection. Static targets and substitutions travel together.
use super::{
    bound_call::Bindings,
    contract::{
        flow_cache::BoundMethod,
        member_applicability::{expand, ReceiverPattern},
        SymbolLookup,
    },
    member_index::MemberNameId,
    trait_graph::{File, Graph, Obligation},
};
use crate::type_checker::core::types::{
    GenericParamKind, Indirection, Lifetime, Mutability, Type, TypeArena, TypeId,
};
use rustc_hash::FxHashSet;

#[path = "trait_obligations.rs"]
mod obligations;
#[path = "trait_qualified_call.rs"]
mod qualified_call;
pub(super) use qualified_call::select as qualified;

pub(super) fn select(
    graph: &Graph,
    file: &File,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    receiver: TypeId,
    name: MemberNameId,
    selector: u32,
    caller: i64,
) -> Result<BoundMethod, ()> {
    let mut visible = FxHashSet::default();
    let mut complete = true;
    let mut scope = Some(*file.selectors.get(&selector).ok_or(())?);
    let mut seen = FxHashSet::default();
    while let Some(id) = scope {
        if !seen.insert(id) {
            return Err(());
        }
        let frame = file.frames.get(&id).ok_or(())?;
        visible.extend(frame.traits.iter().copied());
        complete &= frame.complete;
        scope = frame.parent;
    }
    let assumptions = graph.assumptions(caller, arena)?;
    if super::trace::TRACE_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) {
        crate::tracef!(
            "  METHOD-SCOPE caller={} selector={} receiver={:?} complete={} traits={:?}",
            caller,
            selector,
            arena.get(receiver),
            complete,
            visible
        );
    }
    let mut bases = Vec::new();
    let mut current = expand(lookup, arena, receiver).ok_or(())?;
    for _ in 0..32 {
        if bases.contains(&current) {
            return Err(());
        }
        bases.push(current);
        if super::head_decl::head_decl_id(arena, current)
            .is_some_and(|id| !lookup.declaration_accessible(id))
        {
            return Err(());
        }
        match arena.get(current) {
            Type::Indirect {
                kind: Indirection::Reference(_),
                inner,
                ..
            } => current = inner,
            _ => break,
        }
    }
    let region = Lifetime::Inference {
        owner: caller,
        byte: selector,
    };
    for &base in &bases {
        for candidate in [
            base,
            reference(arena, base, region, Mutability::Shared),
            reference(arena, base, region, Mutability::Mutable),
        ] {
            let mut inherent = Vec::new();
            let mut unknown = false;
            for &self_type in &bases {
                if let Some(owner) = super::head_decl::head_decl_id(arena, self_type) {
                    for &member in lookup.member_index().ok_or(())?.candidates(owner, name) {
                        let Some(info) = lookup.canonical_type_info(member) else {
                            unknown = true;
                            continue;
                        };
                        let Some(pattern) = info.receiver_type_id else {
                            continue;
                        };
                        if super::trace::TRACE_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) {
                            crate::tracef!(
                                "  METHOD-CANDIDATE owner={} method={} actual={:?} signature={:?}",
                                owner,
                                member,
                                arena.get(candidate),
                                arena.get(pattern)
                            );
                        }
                        let mut bindings = super::bound_call::receiver_bindings(
                            lookup,
                            arena,
                            self_type,
                            Some(owner),
                        );
                        if let Some(pattern) = lookup.member_pattern(member) {
                            match pattern.bindings(lookup, arena, self_type) {
                                Ok(Some(env)) => bindings.extend(env),
                                Ok(None) => continue,
                                Err(()) => {
                                    unknown = true;
                                    continue;
                                }
                            }
                        }
                        match receiver_match(lookup, arena, member, pattern, candidate, &bindings) {
                            Ok(Some(env)) => {
                                if !lookup.declaration_accessible(member) {
                                    return Err(());
                                }
                                bindings.extend(env);
                                inherent.push(BoundMethod {
                                    declaration: member,
                                    receiver: self_type,
                                    adjusted: candidate,
                                    bindings,
                                });
                            }
                            Ok(None) => {}
                            Err(()) => unknown = true,
                        }
                    }
                }
            }
            if unknown || inherent.len() > 1 {
                return Err(());
            }
            if let Some(found) = inherent.pop() {
                return Ok(found);
            }
            // Generic-bound candidates precede other visible traits at this
            // exact receiver candidate, not at a later auto-borrow stage.
            let mut bounded = Vec::new();
            for bound in &assumptions {
                if !bases.contains(&bound.subject) {
                    continue;
                }
                add_trait(
                    graph,
                    lookup,
                    arena,
                    *bound,
                    name,
                    candidate,
                    &assumptions,
                    &mut bounded,
                    &mut unknown,
                );
            }
            dedup(&mut bounded);
            if unknown || bounded.len() > 1 {
                return Err(());
            }
            if let Some(found) = bounded.pop() {
                return Ok(found);
            }
            let mut traits = Vec::new();
            for id in &visible {
                let Some(definition) = graph.definitions.get(id) else {
                    unknown = true;
                    continue;
                };
                if !definition.members.contains_key(&name) {
                    continue;
                }
                for implementation in graph
                    .implementations
                    .get(&Some(*id))
                    .into_iter()
                    .flatten()
                    .chain(graph.implementations.get(&None).into_iter().flatten())
                {
                    for &self_type in &bases {
                        let bindings =
                            match implementation.receiver.bindings(lookup, arena, self_type) {
                                Ok(Some(bindings)) => bindings,
                                Ok(None) => continue,
                                Err(()) => {
                                    unknown = true;
                                    continue;
                                }
                            };
                        let bound = Obligation {
                            subject: self_type,
                            trait_type: super::contract::generic_return::substitute(
                                arena,
                                implementation.trait_type,
                                &bindings,
                            ),
                        };
                        // Check receiver shape before obligations: a candidate at
                        // a later borrow stage cannot poison an earlier winner.
                        let mut matches = Vec::new();
                        add_trait(
                            graph,
                            lookup,
                            arena,
                            bound,
                            name,
                            candidate,
                            &assumptions,
                            &mut matches,
                            &mut unknown,
                        );
                        if matches.is_empty() {
                            continue;
                        }
                        if !implementation.enabled {
                            unknown = true;
                            continue;
                        }
                        if implementation.negative {
                            unknown = true;
                            continue;
                        }
                        match obligations::all(
                            graph,
                            lookup,
                            arena,
                            implementation.declaration,
                            &bindings,
                            &assumptions,
                            &mut FxHashSet::default(),
                            &mut 4096,
                        ) {
                            Ok(true) => traits.extend(matches),
                            Ok(false) => {}
                            Err(()) => unknown = true,
                        }
                    }
                }
            }
            // Do not deduplicate distinct implementations: applicability itself
            // must be unique, even when their static declaration target agrees.
            if !complete || unknown || traits.len() > 1 {
                return Err(());
            }
            if let Some(found) = traits.pop() {
                return Ok(found);
            }
        }
    }
    Err(())
}

fn dedup(items: &mut Vec<BoundMethod>) {
    let mut unique: Vec<BoundMethod> = Vec::new();
    for item in items.drain(..) {
        if !unique.iter().any(|old| {
            old.declaration == item.declaration
                && old.receiver == item.receiver
                && old.bindings == item.bindings
        }) {
            unique.push(item);
        }
    }
    *items = unique;
}

fn add_trait(
    graph: &Graph,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    bound: Obligation,
    name: MemberNameId,
    candidate: TypeId,
    assumptions: &[Obligation],
    out: &mut Vec<BoundMethod>,
    unknown: &mut bool,
) {
    let Some(id) = super::head_decl::head_decl_id(arena, bound.trait_type) else {
        *unknown = true;
        return;
    };
    let Some(definition) = graph.definitions.get(&id) else {
        *unknown = true;
        return;
    };
    let Some(members) = definition.members.get(&name) else {
        return;
    };
    if !definition.enabled {
        *unknown = true;
        return;
    }
    let Some(bindings) = graph.bindings(id, bound.subject, bound.trait_type, arena) else {
        *unknown = true;
        return;
    };
    for &member in members {
        let Some(pattern) = lookup
            .canonical_type_info(member)
            .and_then(|i| i.receiver_type_id)
        else {
            continue;
        };
        match receiver_match(lookup, arena, member, pattern, candidate, &bindings) {
            Ok(Some(env)) => {
                let mut bindings = bindings.clone();
                bindings.extend(env);
                match obligations::all(
                    graph,
                    lookup,
                    arena,
                    member,
                    &bindings,
                    assumptions,
                    &mut FxHashSet::default(),
                    &mut 4096,
                ) {
                    Ok(true) => out.push(BoundMethod {
                        declaration: member,
                        receiver: bound.subject,
                        adjusted: candidate,
                        bindings,
                    }),
                    Ok(false) => {}
                    Err(()) => *unknown = true,
                }
            }
            Ok(None) => {}
            Err(()) => *unknown = true,
        }
    }
}

fn receiver_match(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    member: i64,
    pattern: TypeId,
    actual: TypeId,
    bindings: &Bindings,
) -> Result<Option<Bindings>, ()> {
    let pattern = super::contract::generic_return::substitute(arena, pattern, bindings);
    let pattern = expand(lookup, arena, pattern).ok_or(())?;
    // A rigid generic parameter is not equal to a reference containing that
    // parameter. The following auto-borrow candidate is considered separately.
    if matches!(
        (arena.get(pattern), arena.get(actual)),
        (Type::Indirect { .. }, Type::Generic { .. })
            | (Type::Generic { .. }, Type::Indirect { .. })
    ) {
        return Ok(None);
    }
    let info = lookup.canonical_type_info(member).ok_or(())?;
    let parameters = match arena.get(pattern) {
        Type::Indirect {
            kind: Indirection::Reference(Lifetime::Parameter(p)),
            ..
        } if arena.generic_kind(p) == GenericParamKind::Lifetime
            && (info.generic_param_ids.contains(&p)
                || info.elided_input_params.iter().any(|&(_, _, q)| p == q)) =>
        {
            vec![p]
        }
        _ => Vec::new(),
    };
    ReceiverPattern {
        ty: pattern,
        parameters,
    }
    .bindings(lookup, arena, actual)
}

fn reference(arena: &TypeArena, inner: TypeId, region: Lifetime, mutability: Mutability) -> TypeId {
    arena.intern(Type::Indirect {
        kind: Indirection::Reference(region),
        mutability,
        inner,
    })
}

#[cfg(test)]
#[path = "trait_selection_tests.rs"]
mod tests;
