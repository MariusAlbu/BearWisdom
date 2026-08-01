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
) {
    if !args.iter().any(|a| matches!(a, CallArg::Lambda { .. })) {
        return;
    }
    let patterns = param_patterns(arena, callee);
    if patterns.is_empty() {
        return;
    }
    let mut env = receiver_env(lookup, arena, receiver, recv_id);
    env.extend(arg_env.iter().map(|(k, v)| (k.clone(), *v)));
    let open = bindable_params(lookup, callee);
    for (i, arg) in args.iter().enumerate() {
        let CallArg::Lambda { params } = arg else {
            continue;
        };
        let Some(&pattern) = patterns.get(i) else {
            continue;
        };
        let Type::Function {
            params: callback_params,
            ..
        } = arena.get(substitute_env(arena, pattern, &env))
        else {
            continue;
        };
        seed_names(lookup, arena, params, &callback_params, &open);
    }
}

/// Record each named lambda parameter under the callback parameter type at the
/// same position. Both the id and the string cache are written: the string one
/// is what the root resolver's `local_type` path reads.
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
        lookup.record_local_type(name.clone(), arena.format_type(ty));
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
