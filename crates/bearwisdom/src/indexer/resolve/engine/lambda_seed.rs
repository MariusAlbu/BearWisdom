// =============================================================================
// engine/lambda_seed — contextual typing of un-annotated lambda parameters
//
// `arr.map(x => x.foo)` declares nothing about `x`; its type comes from the
// callee's own signature — the callback parameter `map` declares — once the
// receiver's generic arguments are substituted into it. Seeding that type into
// the file-local forward cache is what lets the NEXT ref in the file (`x.foo`)
// root on a real type instead of dying at the lambda parameter.
// =============================================================================

use rustc_hash::{FxHashMap, FxHashSet};

use crate::indexer::resolve::engine::contract::{Symbol, SymbolLookup};
use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::type_checker::profile::language_profile::DelegateShape;
use crate::types::CallArg;

use super::generics::{bindable_params, param_patterns, substitute_env};
use super::substitution::receiver_env;

/// Seed every un-annotated lambda parameter of this call with the type the
/// callee's matching callback parameter declares.
///
/// The callback's declared type is written in the callee's vocabulary
/// (`(value: T, index: number) => U`), so it is rewritten through the
/// receiver's bindings and then the argument-driven ones before its parameter
/// types are read. A callback parameter still naming an open generic is
/// skipped — an unbindable receiver seeds nothing rather than seeding a name.
///
/// Positional throughout: lambda parameter `j` takes callback parameter `j`,
/// and an empty name (a destructuring or rest binding the extractor could not
/// name) is skipped while keeping later positions aligned.
pub(crate) fn seed_lambda_params(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    callee: &Symbol,
    args: &[CallArg],
    receiver: TypeId,
    recv_id: Option<i64>,
    arg_env: &FxHashMap<String, TypeId>,
    delegate_wrappers: &[(&str, DelegateShape)],
) {
    if !args
        .iter()
        .any(|a| matches!(a, CallArg::Lambda { .. } | CallArg::LambdaAt { .. }))
    {
        return;
    }
    let patterns = param_patterns(lookup, arena, callee);
    if patterns.is_empty() {
        return;
    }
    let mut env = receiver_env(lookup, arena, receiver, recv_id);
    merge_argument_bindings(arena, &mut env, arg_env);
    let open = bindable_params(lookup, callee);
    let patterns: Vec<_> = patterns
        .iter()
        .map(|&p| substitute_env(arena, p, &env))
        .collect();
    seed_patterns(lookup, arena, args, &patterns, &open, delegate_wrappers);
}

/// Keep a concrete generic binding learned from the receiver unless argument
/// inference agrees with it. A disagreement means the selected call shape is
/// not coherent enough to contextually type a callback, so leave the generic
/// open and let ordinary resolution continue without this seed.
fn merge_argument_bindings(
    arena: &TypeArena,
    receiver_env: &mut FxHashMap<String, TypeId>,
    arg_env: &FxHashMap<String, TypeId>,
) {
    let unknown = arena.intern(Type::Unknown);
    for (name, &argument) in arg_env {
        match receiver_env.get(name).copied() {
            None => {
                receiver_env.insert(name.clone(), argument);
            }
            Some(receiver) if receiver == argument => {}
            Some(_) => {
                receiver_env.insert(name.clone(), unknown);
            }
        }
    }
}

pub(super) fn seed_patterns(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    args: &[CallArg],
    patterns: &[TypeId],
    open: &FxHashSet<String>,
    delegate_wrappers: &[(&str, DelegateShape)],
) {
    for (i, arg) in args.iter().enumerate() {
        if !matches!(arg, CallArg::Lambda { .. } | CallArg::LambdaAt { .. }) {
            continue;
        }
        let Some(&pattern) = patterns.get(i) else {
            continue;
        };
        crate::tracef!(
            "  CALLBACK argument={:?} pattern={:?}",
            arg,
            arena.get(pattern)
        );
        let Some(callback_params) = callback_param_types(arena, pattern, delegate_wrappers) else {
            continue;
        };
        match arg {
            CallArg::LambdaAt { params } => {
                for (span, &ty) in params.iter().zip(&callback_params) {
                    crate::tracef!(
                        "  CALLBACK parameter={:?} type={:?} open={}",
                        span,
                        arena.get(ty),
                        is_open(arena, ty, open)
                    );
                    if let Some(span) = span.filter(|_| !is_open(arena, ty, &open)) {
                        lookup.record_contextual_type(span, ty);
                    }
                }
            }
            CallArg::Lambda { params } => {
                seed_names(lookup, arena, params, &callback_params, &open)
            }
            _ => {}
        }
    }
}

/// The PARAMETER types of a callback-shaped callee parameter: an inline
/// function type yields its params directly; a nominal DELEGATE wrapper
/// (`Action<T1,T2>`, `Func<T,R>`, peeled through an `Optional`) yields its
/// generic arguments per the profile's declared shape. `None` for anything
/// else — a nominal parameter that is not a declared delegate stays opaque.
fn callback_param_types(
    arena: &TypeArena,
    ty: TypeId,
    delegate_wrappers: &[(&str, DelegateShape)],
) -> Option<Vec<TypeId>> {
    match arena.get(ty) {
        Type::Function { params, .. } => Some(params),
        Type::Callable(c) if c.complete => Some(
            c.parameters
                .into_iter()
                .filter(|p| !p.receiver)
                .map(|p| p.ty)
                .collect(),
        ),
        // `Action<T>?` — the nullable annotation doesn't change the shape.
        Type::Optional(inner) => callback_param_types(arena, inner, delegate_wrappers),
        Type::Apply { base, args } => {
            let Type::Class(head) = arena.get(base) else {
                return None;
            };
            let simple = head.rsplit('.').next().unwrap_or(&head);
            let (_, shape) = delegate_wrappers.iter().find(|(n, _)| *n == simple)?;
            match shape {
                DelegateShape::AllParams => Some(args),
                DelegateShape::LastIsReturn => {
                    let n = args.len().checked_sub(1)?;
                    Some(args[..n].to_vec())
                }
            }
        }
        _ => None,
    }
}

/// Record each named lambda parameter under the callback parameter type at the
/// same position on the legacy language path. Preserve TypeIds without a
/// display-format write that would evict them from the cache.
fn seed_names(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    names: &[String],
    callback_params: &[TypeId],
    open: &FxHashSet<String>,
) {
    for (name, &ty) in names.iter().zip(callback_params.iter()) {
        if name.is_empty() || is_open(arena, ty, open) {
            continue;
        }
        lookup.record_local_type_id(name.clone(), ty);
    }
}

/// True when the type is one the substitution failed to bind — the engine's
/// bailout, or a name that is still one of the callee's own generic parameters.
fn is_open(arena: &TypeArena, ty: TypeId, open: &FxHashSet<String>) -> bool {
    match arena.get(ty) {
        Type::Unknown | Type::Generic { .. } => true,
        Type::Class(name) => open.contains(&name),
        _ => false,
    }
}

#[cfg(test)]
#[path = "lambda_seed_tests.rs"]
mod tests;
