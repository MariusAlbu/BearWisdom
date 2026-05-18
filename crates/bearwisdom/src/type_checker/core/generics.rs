// =============================================================================
// type_checker/core/generics.rs — generic-parameter substitution
//
// Given a TypeId and a binding env (GenericParamId → TypeId), walk the Type
// graph and rewrite every `Type::Generic { param }` occurrence with its bound
// concrete type. Returns the original TypeId unchanged when no substitution
// happens (cheap fast path for the common "type contains no generics" case).
//
// Spec: research/architecture/02-engine-internal-architecture.html § Layer 1
//       research/architecture/04-implementation-phases.html § Phase 2
// =============================================================================

use super::types::{GenericParamId, Type, TypeArena, TypeId};
use rustc_hash::FxHashMap;

/// GenericParamId → TypeId bindings for a single substitution context.
///
/// Created at the call site that knows the binding (e.g. a chain step
/// arriving at `Type::Apply { base: Repository, args: [User] }` binds
/// Repository's declared param `T` → User's TypeId). The env is consumed
/// once per substitution call; nested scopes are modeled by creating a new
/// env, not by stack-pushing — the arena's TypeId equality already gives
/// us the equivalent of scope-correct lookup for free.
#[derive(Debug, Default, Clone)]
pub struct GenericEnv {
    bindings: FxHashMap<GenericParamId, TypeId>,
}

impl GenericEnv {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            bindings: FxHashMap::with_capacity_and_hasher(cap, Default::default()),
        }
    }

    pub fn bind(&mut self, param: GenericParamId, ty: TypeId) {
        self.bindings.insert(param, ty);
    }

    pub fn get(&self, param: GenericParamId) -> Option<TypeId> {
        self.bindings.get(&param).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Positional bind of `params` to `args`. Stops at the shorter list,
    /// matching how generic arguments propagate when fewer args than
    /// declared params are supplied (the rest stay unbound — substitution
    /// leaves them as Generic, which downstream lookups treat as opaque).
    pub fn bind_positional(&mut self, params: &[GenericParamId], args: &[TypeId]) {
        for (param, arg) in params.iter().zip(args.iter()) {
            self.bind(*param, *arg);
        }
    }
}

/// Rewrite `ty` with `env`'s bindings applied. Re-interns into `arena` only
/// when a sub-type changed. Unbound `Generic { param }` slots survive
/// unchanged — the caller decides whether that's a miss or a legitimate
/// pass-through (e.g. nested generic method on an outer-generic class).
pub fn substitute(ty: TypeId, env: &GenericEnv, arena: &mut TypeArena) -> TypeId {
    if env.is_empty() {
        return ty;
    }
    let current = arena.get(ty).clone();
    match current {
        Type::Generic { param } => env.get(param).unwrap_or(ty),
        Type::Apply { base, args } => {
            let new_base = substitute(base, env, arena);
            let mut new_args = Vec::with_capacity(args.len());
            let mut changed = new_base != base;
            for a in args {
                let new_a = substitute(a, env, arena);
                if new_a != a {
                    changed = true;
                }
                new_args.push(new_a);
            }
            if !changed {
                return ty;
            }
            arena.intern(Type::Apply {
                base: new_base,
                args: new_args,
            })
        }
        Type::Function { params, return_ } => {
            let new_return = substitute(return_, env, arena);
            let mut new_params = Vec::with_capacity(params.len());
            let mut changed = new_return != return_;
            for p in params {
                let new_p = substitute(p, env, arena);
                if new_p != p {
                    changed = true;
                }
                new_params.push(new_p);
            }
            if !changed {
                return ty;
            }
            arena.intern(Type::Function {
                params: new_params,
                return_: new_return,
            })
        }
        Type::Tuple(elems) => substitute_seq(ty, &elems, env, arena, Type::Tuple),
        Type::Union(branches) => substitute_seq(ty, &branches, env, arena, Type::Union),
        Type::Intersection(branches) => {
            substitute_seq(ty, &branches, env, arena, Type::Intersection)
        }
        Type::Optional(inner) => substitute_wrap(ty, inner, env, arena, Type::Optional),
        Type::AsyncWrapper(inner) => substitute_wrap(ty, inner, env, arena, Type::AsyncWrapper),
        Type::Iterator(inner) => substitute_wrap(ty, inner, env, arena, Type::Iterator),
        Type::Class(_) | Type::Primitive(_) | Type::Literal(_) | Type::Unknown => ty,
    }
}

fn substitute_seq(
    original: TypeId,
    elems: &[TypeId],
    env: &GenericEnv,
    arena: &mut TypeArena,
    ctor: fn(Vec<TypeId>) -> Type,
) -> TypeId {
    let mut new_elems = Vec::with_capacity(elems.len());
    let mut changed = false;
    for e in elems {
        let new_e = substitute(*e, env, arena);
        if new_e != *e {
            changed = true;
        }
        new_elems.push(new_e);
    }
    if !changed {
        return original;
    }
    arena.intern(ctor(new_elems))
}

fn substitute_wrap(
    original: TypeId,
    inner: TypeId,
    env: &GenericEnv,
    arena: &mut TypeArena,
    ctor: fn(TypeId) -> Type,
) -> TypeId {
    let new_inner = substitute(inner, env, arena);
    if new_inner == inner {
        return original;
    }
    arena.intern(ctor(new_inner))
}

#[cfg(test)]
#[path = "generics_tests.rs"]
mod tests;
