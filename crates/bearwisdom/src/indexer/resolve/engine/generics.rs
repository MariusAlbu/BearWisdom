// =============================================================================
// engine/generics — argument-driven generic binding
//
// The structural inverse of `TypeArena::rebind_class_params`: given a callee's
// declared parameter type (a pattern that may name generic parameters) and the
// resolved type of the argument passed for it, produce the
// `{parameter name → TypeId}` bindings that make the two agree. Substituting
// those into the callee's yield resolves `repo.find(user).name` — `find(x: T): T`
// yields `User` because the ARGUMENT says so, with nothing declared on the
// receiver.
//
// Applied strictly AFTER receiver-driven substitution: a parameter the receiver
// already pinned is concrete by then and no longer matches a pattern name, so an
// argument can only fill a slot the receiver left open, never override one.
// =============================================================================

use rustc_hash::{FxHashMap, FxHashSet};

use crate::indexer::resolve::engine::contract::{
    parse_param_types_from_signature, Symbol, SymbolLookup,
};
use crate::type_checker::core::types::{Type, TypeArena, TypeId};

use super::chain::head_qname;

/// Bind the generic parameters `pattern` names from the matching positions of
/// `actual`, writing into `env`.
///
/// Recurses only into MATCHING constructors — a generic application with the
/// same head and arity, a function type, a tuple of the same length, the same
/// single-inner wrapper — plus one asymmetric case: a pattern-side `Optional`
/// unifies against a bare actual (`x?: T` passed a `User`). Anything else — a
/// union, an intersection, a head or arity mismatch — is a silent no-op that
/// leaves the slot open rather than guessing.
///
/// A slot binds at most once (the first concrete position wins) and never to
/// `Unknown` or to another generic parameter, so an untyped argument can never
/// erase a binding a later argument would have supplied.
pub(crate) fn unify_into(
    arena: &TypeArena,
    pattern: TypeId,
    actual: TypeId,
    params: &FxHashSet<String>,
    env: &mut FxHashMap<String, TypeId>,
) {
    if is_opaque(arena, actual, params) {
        return;
    }
    match (arena.get(pattern), arena.get(actual)) {
        (Type::Class(name), _) => {
            if params.contains(&name) {
                env.entry(name).or_insert(actual);
            }
        }
        (
            Type::Apply {
                base: p_base,
                args: p_args,
            },
            Type::Apply {
                base: a_base,
                args: a_args,
            },
        ) => {
            if p_args.len() != a_args.len()
                || head_qname(arena, p_base) != head_qname(arena, a_base)
            {
                return;
            }
            for (p, a) in p_args.iter().zip(a_args.iter()) {
                unify_into(arena, *p, *a, params, env);
            }
        }
        (
            Type::Function {
                params: p_params,
                return_: p_ret,
            },
            Type::Function {
                params: a_params,
                return_: a_ret,
            },
        ) => {
            for (p, a) in p_params.iter().zip(a_params.iter()) {
                unify_into(arena, *p, *a, params, env);
            }
            unify_into(arena, p_ret, a_ret, params, env);
        }
        (Type::Tuple(p_elems), Type::Tuple(a_elems)) if p_elems.len() == a_elems.len() => {
            for (p, a) in p_elems.iter().zip(a_elems.iter()) {
                unify_into(arena, *p, *a, params, env);
            }
        }
        (Type::Optional(p_in), Type::Optional(a_in))
        | (Type::AsyncWrapper(p_in), Type::AsyncWrapper(a_in))
        | (Type::Iterator(p_in), Type::Iterator(a_in)) => {
            unify_into(arena, p_in, a_in, params, env)
        }
        // `x?: T` given a bare `User`: the nullable wrapper belongs to the
        // parameter, not the argument — unify through it.
        (Type::Optional(p_in), _) => unify_into(arena, p_in, actual, params, env),
        _ => {}
    }
}

/// True when `actual` carries nothing a binding could use: the engine's bailout
/// type, or a still-open generic parameter — binding `T` to `U` would record a
/// name no later substitution can resolve.
fn is_opaque(arena: &TypeArena, actual: TypeId, params: &FxHashSet<String>) -> bool {
    match arena.get(actual) {
        Type::Unknown | Type::Generic { .. } => true,
        Type::Class(name) => params.contains(&name),
        _ => false,
    }
}

/// The generic parameters bindable at a call to `callee`: the ones declared on
/// the callable itself plus the ones its declaring type introduces. A method's
/// parameter can name either (`class Repo<T> { find(x: T, k: K<T>): T }`), and
/// the receiver-first ordering keeps an owner parameter the receiver already
/// pinned out of reach — by then it is no longer a name in the pattern.
pub(crate) fn bindable_params(lookup: &dyn SymbolLookup, callee: &Symbol) -> FxHashSet<String> {
    let mut out: FxHashSet<String> = lookup
        .generic_params_of(callee.id)
        .or_else(|| lookup.generic_params(&callee.qualified_name))
        .unwrap_or_default()
        .into_iter()
        .collect();
    if let Some((owner, _)) = callee.qualified_name.rsplit_once('.') {
        out.extend(lookup.generic_params(owner).unwrap_or_default());
    }
    out
}

/// `callee`'s declared parameter types, interned as canonical TypeIds in
/// declaration order. Parsed from the stored signature — the same text
/// `languages::common::populate_return_type_ids` interns from at extract time —
/// so a callee reached through any index path (parsed batch, DB reload,
/// external cache) exposes the same patterns. Empty when the signature carries
/// no parameter list.
pub(crate) fn param_patterns(arena: &TypeArena, callee: &Symbol) -> Vec<TypeId> {
    callee
        .signature
        .as_deref()
        .and_then(parse_param_types_from_signature)
        .map(|ps| ps.iter().map(|p| arena.intern_type_str(p)).collect())
        .unwrap_or_default()
}

/// The bindings a call's argument types impose on `callee`'s generic
/// parameters. Empty when the call passes no arguments, the callee introduces
/// no parameters, or no argument's type matched a pattern position.
pub(crate) fn bind_arg_generics(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    callee: &Symbol,
    arg_types: &[TypeId],
) -> FxHashMap<String, TypeId> {
    let mut env: FxHashMap<String, TypeId> = FxHashMap::default();
    if arg_types.is_empty() {
        return env;
    }
    let params = bindable_params(lookup, callee);
    if params.is_empty() {
        return env;
    }
    for (pattern, actual) in param_patterns(arena, callee).iter().zip(arg_types.iter()) {
        unify_into(arena, *pattern, *actual, &params, &mut env);
    }
    env
}

/// Rewrite `yielded` through `env`, unchanged when `env` binds nothing.
pub(crate) fn substitute_env(
    arena: &TypeArena,
    yielded: TypeId,
    env: &FxHashMap<String, TypeId>,
) -> TypeId {
    if env.is_empty() {
        yielded
    } else {
        arena.rebind_class_params(yielded, env)
    }
}

/// Fill `yielded`'s still-open generic parameters from the types of the
/// arguments the call passes.
pub(crate) fn fill_yield_from_args(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    callee: &Symbol,
    arg_types: &[TypeId],
    yielded: TypeId,
) -> TypeId {
    let env = bind_arg_generics(lookup, arena, callee, arg_types);
    substitute_env(arena, yielded, &env)
}

#[cfg(test)]
#[path = "generics_tests.rs"]
mod tests;
