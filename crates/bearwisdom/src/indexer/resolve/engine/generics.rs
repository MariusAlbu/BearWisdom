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

use crate::indexer::resolve::engine::contract::{Symbol, SymbolLookup};
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
    lookup: &dyn SymbolLookup,
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
        // A class VALUE against a token-shaped parameter: `inject(TasksService)`
        // with `token: ProviderToken<T>` binds `T` to the INSTANCE the
        // constructor builds. Token evidence is structural — the pattern's
        // declaration (through its alias, on any union arm) declares a
        // construct signature — and exactly one open slot keeps the binding
        // unambiguous. A pattern without that evidence stays a silent no-op:
        // a constructor never structurally matches an applied instance type.
        (
            Type::Apply {
                base: p_base,
                args: p_args,
            },
            Type::Constructor(inner),
        ) => {
            let open: Vec<String> = p_args
                .iter()
                .filter_map(|&a| match arena.get(a) {
                    Type::Class(n) if params.contains(&n) => Some(n),
                    _ => None,
                })
                .collect();
            if let [slot] = &open[..] {
                if pattern_head_constructs(lookup, arena, p_base) {
                    env.entry(slot.clone()).or_insert(inner);
                }
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
                unify_into(lookup, arena, *p, *a, params, env);
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
                unify_into(lookup, arena, *p, *a, params, env);
            }
            unify_into(lookup, arena, p_ret, a_ret, params, env);
        }
        (Type::Tuple(p_elems), Type::Tuple(a_elems)) if p_elems.len() == a_elems.len() => {
            for (p, a) in p_elems.iter().zip(a_elems.iter()) {
                unify_into(lookup, arena, *p, *a, params, env);
            }
        }
        (Type::Optional(p_in), Type::Optional(a_in))
        | (Type::AsyncWrapper(p_in), Type::AsyncWrapper(a_in))
        | (Type::Iterator(p_in), Type::Iterator(a_in)) => {
            unify_into(lookup, arena, p_in, a_in, params, env)
        }
        // `x?: T` given a bare `User`: the nullable wrapper belongs to the
        // parameter, not the argument — unify through it.
        (Type::Optional(p_in), _) => unify_into(lookup, arena, p_in, actual, params, env),
        _ => {}
    }
}

/// `true` when the type a pattern head names declares a construct signature —
/// directly, through its alias, or on any arm of a union alias. The evidence a
/// generic position carries the constructed INSTANCE (`Type<T>` is
/// `new (...) => T`), read structurally off the index: every same-simple-name
/// type candidate is consulted, since a signature-sourced head carries no
/// package qualifier.
fn pattern_head_constructs(lookup: &dyn SymbolLookup, arena: &TypeArena, base: TypeId) -> bool {
    let Some(head) = head_qname(arena, base) else {
        return false;
    };
    let simple = head.rsplit('.').next().unwrap_or(&head);
    for cand in lookup.types_by_name(simple).iter() {
        if declares_constructor(lookup, cand) {
            return true;
        }
        // Follow the head's alias one level: any arm of a union
        // (`ProviderToken<T> = Type<T> | …`) or the root of a direct redirect
        // (`Token<T> = Type<T>`) carrying a construct signature counts.
        let target = lookup
            .alias_target_by_id(cand.id)
            .or_else(|| lookup.alias_target(&cand.qualified_name));
        let arm_ids: Vec<TypeId> = match target {
            Some(crate::types::AliasTargetIds::Union(arms)) => arms.clone(),
            Some(crate::types::AliasTargetIds::Application { root, .. }) => vec![*root],
            _ => Vec::new(),
        };
        for arm in arm_ids {
            let Some(arm_head) = head_qname(arena, arm) else {
                continue;
            };
            let arm_simple = arm_head.rsplit('.').next().unwrap_or(&arm_head);
            if lookup
                .types_by_name(arm_simple)
                .iter()
                .any(|s| declares_constructor(lookup, s))
            {
                return true;
            }
        }
    }
    false
}

/// `true` when `sym` declares a construct-signature member (`new (...): T`).
fn declares_constructor(lookup: &dyn SymbolLookup, sym: &Symbol) -> bool {
    lookup
        .members_of_id(sym.id)
        .iter()
        .chain(lookup.members_of(&sym.qualified_name).iter())
        .any(|m| m.kind == "constructor")
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

/// Read the callee's source-bound parameter TypeIds by declaration identity.
/// Only unmigrated input without canonical parameter metadata parses the legacy
/// stored signature. Persisted source-bound lists take exactly the same ID path.
pub(crate) fn param_patterns(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    callee: &Symbol,
) -> Vec<TypeId> {
    if let Some(patterns) = lookup
        .canonical_type_info(callee.id)
        .and_then(|info| info.parameter_type_ids.as_ref())
    {
        return patterns.clone();
    }
    let lang = lang_for_symbol_path(&callee.file_path);
    callee
        .signature
        .as_deref()
        .and_then(|s| {
            crate::indexer::resolve::engine::contract::chain_walker::parse_param_types_from_signature_for_lang(s, lang)
        })
        .map(|ps| ps.iter().map(|p| arena.intern_type_str(p)).collect())
        .unwrap_or_default()
}

/// The language whose signature SHAPE a symbol's declaration carries, derived
/// from the declaring file: registry extension table for real files, the
/// virtual-scheme owner for demand-index entries. `""` (the colon-shaped
/// default) when neither identifies it.
fn lang_for_symbol_path(path: &str) -> &'static str {
    if let Some(lang) = crate::ecosystem::externals::language_for_virtual_path(path) {
        return lang;
    }
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    crate::languages::default_registry()
        .language_by_extension(name)
        .unwrap_or("")
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
    for (pattern, actual) in param_patterns(lookup, arena, callee)
        .iter()
        .zip(arg_types.iter())
    {
        unify_into(lookup, arena, *pattern, *actual, &params, &mut env);
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
    if let Some(env) = super::bound_call::environment(
        lookup,
        arena,
        callee,
        arena.intern(Type::Unknown),
        None,
        &[],
        arg_types,
    ) {
        return super::contract::generic_return::substitute(arena, yielded, &env);
    }
    let env = bind_arg_generics(lookup, arena, callee, arg_types);
    substitute_env(arena, yielded, &env)
}

#[cfg(test)]
#[path = "generics_tests.rs"]
mod tests;
