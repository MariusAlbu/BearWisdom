//! Call instantiation keeps method receiver evidence separate from arguments.
use super::*;
use crate::indexer::resolve::engine::{bound_call, contract::generic_return, lambda_seed};
use crate::type_checker::profile::language_profile::DelegateShape;
use crate::types::CallArg;

/// Owned auto-borrow evidence is allocated at the source call, not copied from
/// an ordinary argument or invented as a generic declaration parameter.
pub(super) fn borrow_at_selector(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    callee: &Symbol,
    receiver: TypeId,
    projected: Option<TypeId>,
    selector: u32,
) -> Option<TypeId> {
    use crate::type_checker::core::types::{Indirection, Lifetime};
    if projected.is_some() {
        return projected;
    }
    let info = lookup.member_info(arena, receiver, callee.id)?;
    let pattern = info.receiver_type_id?;
    let bindings = bound_call::receiver_bindings(lookup, arena, receiver, None);
    let pattern = generic_return::substitute(arena, pattern, &bindings);
    let pattern = bound_call::member_yield(lookup, arena, callee.id, receiver, pattern);
    let Type::Indirect {
        kind: Indirection::Reference(Lifetime::Parameter(region)),
        mutability,
        inner,
    } = arena.get(pattern)
    else {
        return None;
    };
    if !info.generic_param_ids.contains(&region)
        && !info
            .elided_input_params
            .iter()
            .any(|&(_, _, p)| p == region)
    {
        return None;
    }
    let inner = super::super::contract::member_applicability::expand(lookup, arena, inner)?;
    let exact = super::super::contract::member_applicability::ReceiverPattern {
        ty: inner,
        parameters: vec![],
    };
    if !matches!(exact.bindings(lookup, arena, receiver), Ok(Some(_))) {
        return None;
    }
    let region = arena.region(lookup.method_call_region(selector)?);
    if !matches!(region, Lifetime::Inference { .. }) {
        return None;
    }
    Some(arena.intern(Type::Indirect {
        kind: Indirection::Reference(region),
        mutability,
        inner: receiver,
    }))
}

pub(crate) fn apply_call_args(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    callee: &Symbol,
    selector: u32,
    args: &[CallArg],
    explicit: &[TypeId],
    receiver: TypeId,
    recv_id: Option<i64>,
    yielded: Option<TypeId>,
    wrappers: &[(&str, DelegateShape)],
) -> Option<TypeId> {
    apply_with_receiver(
        lookup, arena, callee, selector, args, explicit, receiver, recv_id, yielded, wrappers, None,
    )
}

pub(super) fn apply_with_receiver(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    callee: &Symbol,
    selector: u32,
    args: &[CallArg],
    explicit: &[TypeId],
    receiver: TypeId,
    recv_id: Option<i64>,
    yielded: Option<TypeId>,
    wrappers: &[(&str, DelegateShape)],
    borrowed: Option<TypeId>,
) -> Option<TypeId> {
    let source_owned = lookup.source_call_arguments(selector).is_some();
    let args = super::super::arg_types::at(lookup, selector, args)?;
    // Zero ordinary arguments still require receiver-region substitution.
    let bound = lookup
        .member_info(arena, receiver, callee.id)
        .is_some_and(|info| info.receiver_type_id.is_some());
    if args.is_empty() && !bound && !source_owned {
        return yielded;
    }
    let callee = if bound || source_owned {
        callee.clone()
    } else {
        select_overload_for_args(lookup, arena, callee, args)
    };
    let actual = resolve_arg_types(lookup, arena, args);
    if let Some(env) = bound_call::environment_with_receiver(
        lookup, arena, &callee, receiver, recv_id, explicit, &actual, borrowed,
    ) {
        let rewrite = |ty| generic_return::substitute(arena, ty, &env);
        let patterns = lookup
            .projected_member_info(arena, receiver, callee.id)
            .and_then(|info| info.parameter_type_ids.clone())
            .unwrap_or_else(|| param_patterns(lookup, arena, &callee));
        let patterns: Vec<_> = patterns.into_iter().map(rewrite).collect();
        lambda_seed::seed_patterns(
            lookup,
            arena,
            args,
            &patterns,
            &Default::default(),
            wrappers,
        );
        return yielded.map(rewrite);
    }
    if source_owned {
        return None;
    }
    let env = bind_arg_generics(lookup, arena, &callee, &actual);
    seed_lambda_params(
        lookup, arena, &callee, args, receiver, recv_id, &env, wrappers,
    );
    yielded.map(|y| substitute_env(arena, y, &env))
}

#[cfg(test)]
#[path = "chain_call_tests.rs"]
mod tests;
