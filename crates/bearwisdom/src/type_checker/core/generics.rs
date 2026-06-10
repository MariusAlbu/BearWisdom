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
use rustc_hash::{FxHashMap, FxHashSet};

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
pub fn substitute(ty: TypeId, env: &GenericEnv, arena: &TypeArena) -> TypeId {
    if env.is_empty() {
        return ty;
    }
    let current = arena.get(ty);
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
    arena: &TypeArena,
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
    arena: &TypeArena,
    ctor: fn(TypeId) -> Type,
) -> TypeId {
    let new_inner = substitute(inner, env, arena);
    if new_inner == inner {
        return original;
    }
    arena.intern(ctor(new_inner))
}

/// One-directional structural unification of a declared parameter type
/// `param` against a resolved argument type `arg`, recording any discovered
/// `Generic` bindings into `env`. The inverse of `substitute`: where
/// `substitute` consumes an env to rewrite generics, this *produces* env
/// entries by matching a generic-bearing pattern against a concrete type.
///
/// Only `Generic` slots whose id is in `bindable` may bind. Those are the
/// callee's OWN type parameters; the receiver/owner's parameters are bound
/// authoritatively by the receiver type (see `ChainWalker::bind_apply_args`)
/// and must never be rebound from an argument, so they are excluded from
/// `bindable` and left untouched here.
///
/// Conservative throughout — it widens what resolves, never narrows wrongly:
///   - a slot binds at most once; an existing binding is never overwritten,
///     so the first concrete position wins across repeated calls;
///   - `arg` of `Unknown` binds nothing, so a later concrete position can
///     still bind the slot and an `Unknown` never clobbers a real type;
///   - recursion descends only into matching constructors (same `Apply` base,
///     same arity); a shape mismatch is a silent no-op, leaving the slot
///     `Generic` (opaque) — the sound "couldn't infer" state.
///
/// `Union` / `Intersection` params are intentionally not unified: which
/// branch a concrete arg corresponds to is ambiguous, so binding from one
/// would be a guess.
pub fn unify_into(
    param: TypeId,
    arg: TypeId,
    bindable: &FxHashSet<GenericParamId>,
    env: &mut GenericEnv,
    arena: &TypeArena,
) {
    match (arena.get(param), arena.get(arg)) {
        // A bindable type-parameter slot ⇄ a concrete arg: bind it (once,
        // and never to `Unknown`).
        (Type::Generic { param: p }, arg_ty) => {
            if bindable.contains(&p) && env.get(p).is_none() && !matches!(arg_ty, Type::Unknown) {
                env.bind(p, arg);
            }
        }
        // Same generic constructor (`Array<T>` ⇄ `Array<User>`): unify the
        // type arguments positionally. A differing base or arity learns
        // nothing.
        (Type::Apply { base: bp, args: ap }, Type::Apply { base: ba, args: aa }) => {
            if bp == ba && ap.len() == aa.len() {
                for (p, a) in ap.iter().zip(aa.iter()) {
                    unify_into(*p, *a, bindable, env, arena);
                }
            }
        }
        // Callback parameter (`(x: T) => U` ⇄ `(x: User) => Account`): unify
        // each parameter position and the return.
        (
            Type::Function {
                params: pp,
                return_: pr,
            },
            Type::Function {
                params: ap,
                return_: ar,
            },
        ) => {
            for (p, a) in pp.iter().zip(ap.iter()) {
                unify_into(*p, *a, bindable, env, arena);
            }
            unify_into(pr, ar, bindable, env, arena);
        }
        // Matching single-inner wrappers: unify the inner type.
        (Type::Optional(pi), Type::Optional(ai))
        | (Type::AsyncWrapper(pi), Type::AsyncWrapper(ai))
        | (Type::Iterator(pi), Type::Iterator(ai)) => {
            unify_into(pi, ai, bindable, env, arena);
        }
        // Optional param against a bare (non-optional) arg (`x?: T` called
        // with a plain `T`): peel the param-side Optional and unify the inner.
        (Type::Optional(pi), _) => unify_into(pi, arg, bindable, env, arena),
        // Tuple of equal arity: unify element-wise.
        (Type::Tuple(pe), Type::Tuple(ae)) if pe.len() == ae.len() => {
            for (p, a) in pe.iter().zip(ae.iter()) {
                unify_into(*p, *a, bindable, env, arena);
            }
        }
        // Concrete-vs-concrete, shape mismatch, or a Union/Intersection on
        // either side: nothing to infer.
        _ => {}
    }
}

#[cfg(test)]
#[path = "generics_tests.rs"]
mod tests;
