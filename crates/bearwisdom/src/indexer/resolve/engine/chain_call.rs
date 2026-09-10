//! Call instantiation keeps method receiver evidence separate from arguments.
use super::*;
use crate::indexer::resolve::engine::{
    bound_call,
    contract::{generic_return, signature_generic_params},
    lambda_seed,
};
use crate::type_checker::{core::types::Type, profile::language_profile::DelegateShape};
use crate::types::CallArg;
use rustc_hash::{FxHashMap, FxHashSet};

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
    if args.is_empty() && explicit.is_empty() && !bound && !source_owned {
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
    let Some(env) = legacy_callback_env(lookup, arena, &callee, &actual, explicit) else {
        return None;
    };
    if env.callback_seedable {
        if let Some(callback_callee) =
            uniquely_contextual_callback_callee(lookup, arena, &callee, args, wrappers)
        {
            let callback_env = if callback_callee.id == callee.id {
                Some(env.clone())
            } else {
                legacy_callback_env(lookup, arena, &callback_callee, &actual, explicit)
            };
            if let Some(callback_env) = callback_env.filter(|env| env.callback_seedable) {
                seed_lambda_params(
                    lookup,
                    arena,
                    &callback_callee,
                    args,
                    receiver,
                    recv_id,
                    &callback_env.bindings,
                    wrappers,
                );
            }
        }
    }
    yielded.map(|y| substitute_env(arena, y, &env.bindings))
}

/// The legacy call path has only display call arguments, so it builds its own
/// generic environment. Explicit method arguments participate in both yielded
/// return substitution and callback patterns, which still name the callee's
/// generics.
///
/// Explicit arguments override inference for the supplied declaration slots,
/// matching the call-site generic contract used by chain segments. An excess
/// argument cannot be matched to any declared slot, so the legacy call is
/// declined rather than contextually typing an invalid callback.
fn legacy_callback_env(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    callee: &Symbol,
    actual: &[TypeId],
    explicit: &[TypeId],
) -> Option<LegacyCallbackEnv> {
    let mut env = bind_arg_generics(lookup, arena, callee, actual);
    let params = lookup
        .generic_params_of(callee.id)
        .or_else(|| lookup.generic_params(&callee.qualified_name))
        .map(LegacyGenericParams::Metadata)
        .or_else(|| {
            callee.signature.as_deref().and_then(|signature| {
                let params: Vec<_> = signature_generic_params(signature, &callee.name)
                    .into_iter()
                    .map(|(name, _, _)| name)
                    .collect();
                (!params.is_empty()).then_some(LegacyGenericParams::Signature(params))
            })
        });
    let Some(params) = params else {
        return Some(LegacyCallbackEnv {
            bindings: env,
            // No declaration clause means explicit values cannot be assigned
            // positionally. Preserve legacy yield inference, while preventing
            // an open callback generic from entering the local cache.
            callback_seedable: explicit.is_empty(),
        });
    };
    let (params, from_metadata) = match params {
        LegacyGenericParams::Metadata(params) => (params, true),
        LegacyGenericParams::Signature(params) => (params, false),
    };
    if explicit.len() > params.len() {
        return None;
    }
    for (name, &argument) in params.iter().zip(explicit) {
        env.insert(name.clone(), argument);
    }
    Some(LegacyCallbackEnv {
        bindings: env,
        // A signature clause safely assigns explicit slots, but cannot make
        // argument inference identify the remaining parameter names.
        callback_seedable: from_metadata || explicit.len() == params.len(),
    })
}

#[derive(Clone)]
struct LegacyCallbackEnv {
    bindings: FxHashMap<String, TypeId>,
    callback_seedable: bool,
}

enum LegacyGenericParams {
    Metadata(Vec<String>),
    Signature(Vec<String>),
}

/// The one exact-arity overload whose callback positions can contextually type
/// this call's lambdas. Legacy extraction has no call-site overload identity,
/// so a tie must abstain rather than write a type from an arbitrary sibling.
///
/// This is deliberately separate from `select_overload_for_args`: that helper
/// continues to select the legacy callee for generic return inference. Only
/// contextual lambda seeding requires this stronger uniqueness proof.
#[cfg(test)]
pub(super) fn _test_uniquely_contextual_callback_callee(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    callee: &Symbol,
    args: &[CallArg],
    wrappers: &[(&str, DelegateShape)],
) -> Option<Symbol> {
    uniquely_contextual_callback_callee(lookup, arena, callee, args, wrappers)
}

fn uniquely_contextual_callback_callee(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    callee: &Symbol,
    args: &[CallArg],
    wrappers: &[(&str, DelegateShape)],
) -> Option<Symbol> {
    if !args
        .iter()
        .any(|arg| matches!(arg, CallArg::Lambda { .. } | CallArg::LambdaAt { .. }))
    {
        return None;
    }

    let mut candidates = vec![callee.clone()];
    candidates.extend(
        lookup
            .all_by_qualified_name(&callee.qualified_name)
            .into_iter()
            .cloned(),
    );

    let mut seen = FxHashSet::default();
    let mut match_ = None;
    for candidate in candidates {
        if !seen.insert(candidate.id) {
            continue;
        }
        let patterns = param_patterns(lookup, arena, &candidate);
        if patterns.len() != args.len()
            || !args.iter().zip(&patterns).all(|(arg, &pattern)| {
                !matches!(arg, CallArg::Lambda { .. } | CallArg::LambdaAt { .. })
                    || callback_pattern(arena, pattern, wrappers)
            })
        {
            continue;
        }
        if match_.replace(candidate).is_some() {
            return None;
        }
    }
    match_
}

fn callback_pattern(
    arena: &TypeArena,
    pattern: TypeId,
    wrappers: &[(&str, DelegateShape)],
) -> bool {
    match arena.get(pattern) {
        Type::Function { .. } => true,
        Type::Callable(callable) => callable.complete,
        Type::Optional(inner) => callback_pattern(arena, inner, wrappers),
        Type::Apply { base, .. } => {
            let Type::Class(head) = arena.get(base) else {
                return false;
            };
            lambda_seed::delegate_wrapper_shape(&head, wrappers).is_some()
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "chain_call_tests.rs"]
mod tests;
