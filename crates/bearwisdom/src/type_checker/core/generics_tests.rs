// =============================================================================
// type_checker/core/generics_tests.rs — Unit tests for substitute / GenericEnv.
// =============================================================================

use super::*;
use crate::type_checker::core::types::{
    GenericParamData, LitValue, PrimKind, Type, TypeArena,
};

fn make_param(arena: &TypeArena, name: &str) -> GenericParamId {
    arena.intern_generic(GenericParamData {
        name: name.to_string(),
        owner_symbol_index: 0,
        bound: None,
    })
}

#[test]
fn substitute_returns_input_unchanged_when_env_empty() {
    let mut arena = TypeArena::new();
    let cls = arena.class("User");
    let env = GenericEnv::new();
    let out = substitute(cls, &env, &mut arena);
    assert_eq!(out, cls);
}

#[test]
fn substitute_replaces_bare_generic() {
    let mut arena = TypeArena::new();
    let t_param = make_param(&mut arena, "T");
    let generic_ty = arena.intern(Type::Generic { param: t_param });
    let user_ty = arena.class("User");

    let mut env = GenericEnv::new();
    env.bind(t_param, user_ty);

    let out = substitute(generic_ty, &env, &mut arena);
    assert_eq!(out, user_ty);
}

#[test]
fn unbound_generic_survives() {
    let mut arena = TypeArena::new();
    let t_param = make_param(&mut arena, "T");
    let generic_ty = arena.intern(Type::Generic { param: t_param });

    let env = GenericEnv::new();
    let out = substitute(generic_ty, &env, &mut arena);
    // With empty env we short-circuit and return the input directly.
    assert_eq!(out, generic_ty);

    // With a non-empty env that doesn't bind T, we still leave T alone.
    let u_param = make_param(&mut arena, "U");
    let user_ty = arena.class("User");
    let mut env = GenericEnv::new();
    env.bind(u_param, user_ty);
    let out = substitute(generic_ty, &env, &mut arena);
    assert_eq!(out, generic_ty);
}

#[test]
fn substitute_inside_apply_args() {
    let mut arena = TypeArena::new();
    let t_param = make_param(&mut arena, "T");
    let generic_t = arena.intern(Type::Generic { param: t_param });
    let list = arena.class("List");
    let apply_t = arena.intern(Type::Apply {
        base: list,
        args: vec![generic_t],
    });
    let user_ty = arena.class("User");

    let mut env = GenericEnv::new();
    env.bind(t_param, user_ty);

    let out = substitute(apply_t, &env, &mut arena);
    match arena.get(out) {
        Type::Apply { base, args } => {
            assert_eq!(base, list);
            assert_eq!(args, vec![user_ty]);
        }
        other => panic!("expected Apply, got {other:?}"),
    }
}

#[test]
fn substitute_inside_nested_apply() {
    // Apply<Map, [string, Apply<List, [T]>]> with T → User
    let mut arena = TypeArena::new();
    let t_param = make_param(&mut arena, "T");
    let generic_t = arena.intern(Type::Generic { param: t_param });
    let list = arena.class("List");
    let map = arena.class("Map");
    let string_ty = arena.primitive(PrimKind::Str);

    let inner_apply = arena.intern(Type::Apply {
        base: list,
        args: vec![generic_t],
    });
    let outer_apply = arena.intern(Type::Apply {
        base: map,
        args: vec![string_ty, inner_apply],
    });

    let user_ty = arena.class("User");
    let mut env = GenericEnv::new();
    env.bind(t_param, user_ty);

    let out = substitute(outer_apply, &env, &mut arena);
    match arena.get(out).clone() {
        Type::Apply { base, args } => {
            assert_eq!(base, map);
            assert_eq!(args[0], string_ty);
            match arena.get(args[1]) {
                Type::Apply {
                    base: inner_base,
                    args: inner_args,
                } => {
                    assert_eq!(inner_base, list);
                    assert_eq!(inner_args, vec![user_ty]);
                }
                other => panic!("expected nested Apply, got {other:?}"),
            }
        }
        other => panic!("expected Apply, got {other:?}"),
    }
}

#[test]
fn substitute_inside_function_signature() {
    // (T) -> T  becomes (User) -> User
    let mut arena = TypeArena::new();
    let t_param = make_param(&mut arena, "T");
    let generic_t = arena.intern(Type::Generic { param: t_param });
    let fn_ty = arena.intern(Type::Function {
        params: vec![generic_t],
        return_: generic_t,
    });

    let user_ty = arena.class("User");
    let mut env = GenericEnv::new();
    env.bind(t_param, user_ty);

    let out = substitute(fn_ty, &env, &mut arena);
    match arena.get(out) {
        Type::Function { params, return_ } => {
            assert_eq!(params, vec![user_ty]);
            assert_eq!(return_, user_ty);
        }
        other => panic!("expected Function, got {other:?}"),
    }
}

#[test]
fn substitute_inside_optional_wrapper() {
    let mut arena = TypeArena::new();
    let t_param = make_param(&mut arena, "T");
    let generic_t = arena.intern(Type::Generic { param: t_param });
    let opt_t = arena.intern(Type::Optional(generic_t));
    let user_ty = arena.class("User");

    let mut env = GenericEnv::new();
    env.bind(t_param, user_ty);

    let out = substitute(opt_t, &env, &mut arena);
    assert_eq!(arena.get(out), Type::Optional(user_ty));
}

#[test]
fn substitute_inside_async_and_iterator_wrappers() {
    let mut arena = TypeArena::new();
    let t_param = make_param(&mut arena, "T");
    let generic_t = arena.intern(Type::Generic { param: t_param });
    let async_t = arena.intern(Type::AsyncWrapper(generic_t));
    let iter_t = arena.intern(Type::Iterator(generic_t));

    let user_ty = arena.class("User");
    let mut env = GenericEnv::new();
    env.bind(t_param, user_ty);

    let async_out = substitute(async_t, &env, &mut arena);
    assert_eq!(arena.get(async_out), Type::AsyncWrapper(user_ty));
    let iter_out = substitute(iter_t, &env, &mut arena);
    assert_eq!(arena.get(iter_out), Type::Iterator(user_ty));
}

#[test]
fn substitute_inside_tuple_union_intersection() {
    let mut arena = TypeArena::new();
    let t_param = make_param(&mut arena, "T");
    let generic_t = arena.intern(Type::Generic { param: t_param });
    let string_ty = arena.primitive(PrimKind::Str);

    let tup = arena.intern(Type::Tuple(vec![generic_t, string_ty]));
    let uni = arena.intern(Type::Union(vec![generic_t, string_ty]));
    let inter = arena.intern(Type::Intersection(vec![generic_t, string_ty]));

    let user_ty = arena.class("User");
    let mut env = GenericEnv::new();
    env.bind(t_param, user_ty);

    let tup_out = substitute(tup, &env, &mut arena);
    assert_eq!(arena.get(tup_out), Type::Tuple(vec![user_ty, string_ty]));
    let uni_out = substitute(uni, &env, &mut arena);
    assert_eq!(arena.get(uni_out), Type::Union(vec![user_ty, string_ty]));
    let inter_out = substitute(inter, &env, &mut arena);
    assert_eq!(
        arena.get(inter_out),
        Type::Intersection(vec![user_ty, string_ty])
    );
}

#[test]
fn substitute_higher_kinded_application() {
    // Higher-kinded resolution needs no dedicated `Type` variant: `F[Item]`,
    // where `F` is a type-constructor parameter, is `Apply { base: Generic(F),
    // args: [Item] }`. Binding `F → List` substitutes the Apply's base, yielding
    // `List[Item]`. Arity (`F[_]`) matters only for kind-checking, not for this.
    let mut arena = TypeArena::new();
    let f_param = make_param(&mut arena, "F");
    let generic_f = arena.intern(Type::Generic { param: f_param });
    let item = arena.class("Item");
    let list = arena.class("List");
    let hkt = arena.intern(Type::Apply {
        base: generic_f,
        args: vec![item],
    });

    let mut env = GenericEnv::new();
    env.bind(f_param, list);

    let out = substitute(hkt, &env, &mut arena);
    assert_eq!(
        arena.get(out),
        Type::Apply {
            base: list,
            args: vec![item]
        }
    );
}

#[test]
fn substitute_leaves_class_primitive_literal_unknown_untouched() {
    let mut arena = TypeArena::new();
    let t_param = make_param(&mut arena, "T");
    let user_ty = arena.class("User");
    let int_ty = arena.primitive(PrimKind::Int);
    let lit_ty = arena.intern(Type::Literal(LitValue::Str("foo".into())));
    let unk_ty = arena.intern(Type::Unknown);

    let mut env = GenericEnv::new();
    env.bind(t_param, user_ty);

    assert_eq!(substitute(user_ty, &env, &mut arena), user_ty);
    assert_eq!(substitute(int_ty, &env, &mut arena), int_ty);
    assert_eq!(substitute(lit_ty, &env, &mut arena), lit_ty);
    assert_eq!(substitute(unk_ty, &env, &mut arena), unk_ty);
}

#[test]
fn bind_positional_stops_at_shorter_list() {
    let mut arena = TypeArena::new();
    let t = make_param(&mut arena, "T");
    let u = make_param(&mut arena, "U");
    let v = make_param(&mut arena, "V");
    let user = arena.class("User");
    let admin = arena.class("Admin");

    let mut env = GenericEnv::new();
    env.bind_positional(&[t, u, v], &[user, admin]);
    assert_eq!(env.get(t), Some(user));
    assert_eq!(env.get(u), Some(admin));
    assert_eq!(env.get(v), None);
    assert_eq!(env.len(), 2);
}

#[test]
fn substitute_apply_with_no_changes_is_identity() {
    // Apply<List, [User]> with env bound to an unrelated param — no rewrite,
    // result is the same TypeId (no spurious re-intern).
    let mut arena = TypeArena::new();
    let list = arena.class("List");
    let user = arena.class("User");
    let apply = arena.intern(Type::Apply {
        base: list,
        args: vec![user],
    });
    let unrelated = make_param(&mut arena, "X");
    let mut env = GenericEnv::new();
    env.bind(unrelated, user);

    let out = substitute(apply, &env, &mut arena);
    assert_eq!(out, apply);
}
