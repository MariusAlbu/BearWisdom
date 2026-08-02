use rustc_hash::{FxHashMap, FxHashSet};

use super::*;
use crate::indexer::resolve::engine::testkit::{sym, Lookup};
use crate::type_checker::core::types::{Type, TypeArena};

/// `find(x: T): T` declared on `Repo<T>`.
fn find_method() -> Symbol {
    Symbol {
        signature: Some("find(x: T): T".to_string()),
        ..sym(1, "find", "Repo.find", "method", "a.ts")
    }
}

fn params(names: &[&str]) -> FxHashSet<String> {
    names.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn arg_type_binds_the_parameter_it_is_passed_for() {
    let lookup = Lookup::new().with_generics("Repo", &["T"]);
    let arena = lookup.type_arena().unwrap();
    let user = arena.class("User");

    let env = bind_arg_generics(&lookup, arena, &find_method(), &[user]);

    assert_eq!(env.get("T").copied(), Some(user));
}

#[test]
fn yield_fills_from_the_argument_when_the_receiver_left_it_open() {
    let lookup = Lookup::new().with_generics("Repo", &["T"]);
    let arena = lookup.type_arena().unwrap();
    let user = arena.class("User");
    let open_yield = arena.class("T");

    let filled = fill_yield_from_args(&lookup, arena, &find_method(), &[user], open_yield);

    assert_eq!(arena.get(filled), Type::Class("User".to_string()));
}

#[test]
fn a_slot_the_receiver_pinned_is_not_reopened_by_an_argument() {
    let lookup = Lookup::new().with_generics("Repo", &["T"]);
    let arena = lookup.type_arena().unwrap();
    let user = arena.class("User");
    // Receiver substitution ran first: the yield already names `Account`, so no
    // parameter name is left for the argument to bind.
    let pinned_yield = arena.class("Account");

    let filled = fill_yield_from_args(&lookup, arena, &find_method(), &[user], pinned_yield);

    assert_eq!(arena.get(filled), Type::Class("Account".to_string()));
}

#[test]
fn first_concrete_position_wins_and_later_ones_do_not_override() {
    let arena = TypeArena::new();
    let lookup = Lookup::new();
    let p = params(&["T"]);
    let pattern = arena.class("T");
    let first = arena.class("User");
    let second = arena.class("Account");
    let mut env: FxHashMap<String, TypeId> = FxHashMap::default();

    unify_into(&lookup, &arena, pattern, first, &p, &mut env);
    unify_into(&lookup, &arena, pattern, second, &p, &mut env);

    assert_eq!(env.get("T").copied(), Some(first));
}

#[test]
fn an_untyped_argument_leaves_the_slot_open() {
    let arena = TypeArena::new();
    let lookup = Lookup::new();
    let p = params(&["T"]);
    let mut env: FxHashMap<String, TypeId> = FxHashMap::default();

    unify_into(
        &lookup,
        &arena,
        arena.class("T"),
        arena.intern(Type::Unknown),
        &p,
        &mut env,
    );

    assert!(env.is_empty());
}

#[test]
fn an_argument_that_is_itself_a_type_parameter_does_not_bind() {
    let arena = TypeArena::new();
    let lookup = Lookup::new();
    let p = params(&["T", "U"]);
    let mut env: FxHashMap<String, TypeId> = FxHashMap::default();

    unify_into(&lookup, &arena, arena.class("T"), arena.class("U"), &p, &mut env);

    assert!(env.is_empty());
}

#[test]
fn matching_applications_unify_position_by_position() {
    let arena = TypeArena::new();
    let lookup = Lookup::new();
    let p = params(&["T"]);
    let pattern = arena.intern(Type::Apply {
        base: arena.class("Box"),
        args: vec![arena.class("T")],
    });
    let actual = arena.intern(Type::Apply {
        base: arena.class("Box"),
        args: vec![arena.class("User")],
    });
    let mut env: FxHashMap<String, TypeId> = FxHashMap::default();

    unify_into(&lookup, &arena, pattern, actual, &p, &mut env);

    assert_eq!(env.get("T").copied(), Some(arena.class("User")));
}

#[test]
fn a_different_application_head_is_a_silent_no_op() {
    let arena = TypeArena::new();
    let lookup = Lookup::new();
    let p = params(&["T"]);
    let pattern = arena.intern(Type::Apply {
        base: arena.class("Box"),
        args: vec![arena.class("T")],
    });
    let actual = arena.intern(Type::Apply {
        base: arena.class("Rc"),
        args: vec![arena.class("User")],
    });
    let mut env: FxHashMap<String, TypeId> = FxHashMap::default();

    unify_into(&lookup, &arena, pattern, actual, &p, &mut env);

    assert!(env.is_empty());
}

#[test]
fn an_optional_parameter_unifies_against_a_bare_argument() {
    let arena = TypeArena::new();
    let lookup = Lookup::new();
    let p = params(&["T"]);
    let pattern = arena.intern(Type::Optional(arena.class("T")));
    let mut env: FxHashMap<String, TypeId> = FxHashMap::default();

    unify_into(&lookup, &arena, pattern, arena.class("User"), &p, &mut env);

    assert_eq!(env.get("T").copied(), Some(arena.class("User")));
}

#[test]
fn a_union_argument_binds_nothing() {
    let arena = TypeArena::new();
    let lookup = Lookup::new();
    let p = params(&["T"]);
    let actual = arena.intern(Type::Union(vec![arena.class("User"), arena.class("Admin")]));
    let mut env: FxHashMap<String, TypeId> = FxHashMap::default();

    unify_into(
        &lookup,
        &arena,
        arena.intern(Type::Apply {
            base: arena.class("Box"),
            args: vec![arena.class("T")],
        }),
        actual,
        &p,
        &mut env,
    );

    assert!(env.is_empty());
}

#[test]
fn parameter_patterns_come_from_the_stored_signature() {
    let arena = TypeArena::new();
    let lookup = Lookup::new();

    let patterns = param_patterns(&arena, &find_method());

    assert_eq!(patterns.len(), 1);
    assert_eq!(arena.get(patterns[0]), Type::Class("T".to_string()));
}

#[test]
fn a_callee_with_no_generic_parameters_binds_nothing() {
    let lookup = Lookup::new();
    let arena = lookup.type_arena().unwrap();

    let env = bind_arg_generics(&lookup, arena, &find_method(), &[arena.class("User")]);

    assert!(env.is_empty());
}

#[test]
fn the_declaring_types_parameters_are_bindable_from_an_argument() {
    let lookup = Lookup::new().with_generics("Repo", &["T"]);

    let bindable = bindable_params(&lookup, &find_method());

    assert!(bindable.contains("T"));
}

#[test]
fn a_class_value_binds_a_token_patterns_open_slot() {
    // inject(token: Token<T>): T called with the CLASS VALUE `TasksService` —
    // the token's construct signature is the structural evidence its generic
    // position carries the constructed instance, so T binds to the class.
    let lookup = Lookup::new()
        .with(sym(3, "Type", "Type", "interface", "a.ts"))
        .with_member("Type", sym(4, "new", "Type.new", "constructor", "a.ts"))
        .with(sym(5, "Token", "Token", "type_alias", "a.ts"))
        .with_alias(
            "Token",
            crate::types::AliasTarget::Application {
                root: "Type".to_string(),
                args: vec!["T".to_string()],
            },
        )
        .with_generics("inject", &["T"]);
    let inject = Symbol {
        signature: Some("function inject<T>(token: Token<T>): T".to_string()),
        ..sym(1, "inject", "inject", "function", "a.ts")
    };
    let arena = lookup.type_arena().unwrap();
    let instance = arena.class("TasksService");
    let ctor = arena.intern(Type::Constructor(instance));

    let env = bind_arg_generics(&lookup, arena, &inject, &[ctor]);

    assert_eq!(env.get("T").copied(), Some(instance));
}

#[test]
fn a_class_value_does_not_bind_a_pattern_without_construct_evidence() {
    // wrap(x: Wrapper<T>): T called with a class value — `Wrapper` declares no
    // construct signature, so nothing says its generic position is the
    // instance; the slot stays open rather than guessing.
    let lookup = Lookup::new()
        .with(sym(3, "Wrapper", "Wrapper", "interface", "a.ts"))
        .with_generics("wrap", &["T"]);
    let wrap = Symbol {
        signature: Some("function wrap<T>(x: Wrapper<T>): T".to_string()),
        ..sym(1, "wrap", "wrap", "function", "a.ts")
    };
    let arena = lookup.type_arena().unwrap();
    let ctor = arena.intern(Type::Constructor(arena.class("TasksService")));

    let env = bind_arg_generics(&lookup, arena, &wrap, &[ctor]);

    assert!(env.is_empty());
}
