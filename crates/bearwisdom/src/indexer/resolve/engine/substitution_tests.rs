use super::*;
use crate::indexer::resolve::engine::testkit::{sym, Lookup};
use crate::type_checker::core::types::Type;

/// `Repo<User>` as an interned application.
fn repo_of_user(arena: &TypeArena) -> TypeId {
    arena.intern(Type::Apply {
        base: arena.class("Repo"),
        args: vec![arena.class("User")],
    })
}

#[test]
fn receiver_arguments_bind_the_receiver_types_parameters() {
    let lookup = Lookup::new().with_generics("Repo", &["T"]);
    let arena = lookup.type_arena().unwrap();

    let env = receiver_env(&lookup, arena, repo_of_user(arena), None);

    assert_eq!(env.get("T").copied(), Some(arena.class("User")));
}

#[test]
fn a_non_generic_receiver_binds_nothing() {
    let lookup = Lookup::new();
    let arena = lookup.type_arena().unwrap();

    let env = receiver_env(&lookup, arena, arena.class("Repo"), None);

    assert!(env.is_empty());
}

#[test]
fn a_members_yield_substitutes_through_the_receiver() {
    let lookup = Lookup::new().with_generics("Repo", &["T"]);
    let arena = lookup.type_arena().unwrap();

    let out = substitute_through(&lookup, arena, arena.class("T"), repo_of_user(arena), None);

    assert_eq!(arena.get(out), Type::Class("User".to_string()));
}

#[test]
fn a_direct_generic_supertypes_edge_args_bind_its_parameters() {
    // class Child extends Base<User> { }   Base<B> { m(): B }
    let lookup = Lookup::new()
        .with_generics("Base", &["B"])
        .with_parent("Child", "Base")
        .with_parent_args("Child", "Base", &["User"]);
    let arena = lookup.type_arena().unwrap();
    let member = sym(1, "m", "Base.m", "method", "a.ts");

    let out = substitute_supertype_args(
        &lookup,
        arena,
        &member,
        arena.class("B"),
        arena.class("Child"),
        None,
    );

    assert_eq!(arena.get(out), Type::Class("User".to_string()));
}

#[test]
fn a_parameter_threaded_through_two_supertype_hops_still_binds() {
    // class Child<T> extends Mid<T> {}   class Mid<M> extends Base<M> {}
    // interface Base<B> { m(): B }       receiver: Child<User>
    let lookup = Lookup::new()
        .with_generics("Child", &["T"])
        .with_generics("Mid", &["M"])
        .with_generics("Base", &["B"])
        .with_parent("Child", "Mid")
        .with_parent_args("Child", "Mid", &["T"])
        .with_parent("Mid", "Base")
        .with_parent_args("Mid", "Base", &["M"]);
    let arena = lookup.type_arena().unwrap();
    let child_of_user = arena.intern(Type::Apply {
        base: arena.class("Child"),
        args: vec![arena.class("User")],
    });
    let member = sym(1, "m", "Base.m", "method", "a.ts");

    let out = substitute_supertype_args(
        &lookup,
        arena,
        &member,
        arena.class("B"),
        child_of_user,
        None,
    );

    assert_eq!(arena.get(out), Type::Class("User".to_string()));
}

#[test]
fn a_member_declared_on_the_receiver_itself_is_left_to_receiver_substitution() {
    let lookup = Lookup::new().with_generics("Repo", &["T"]);
    let arena = lookup.type_arena().unwrap();
    let member = sym(1, "find", "Repo.find", "method", "a.ts");

    let out = substitute_supertype_args(
        &lookup,
        arena,
        &member,
        arena.class("T"),
        repo_of_user(arena),
        None,
    );

    assert_eq!(arena.get(out), Type::Class("T".to_string()));
}

#[test]
fn an_unreachable_declaring_type_leaves_the_yield_untouched() {
    let lookup = Lookup::new()
        .with_generics("Base", &["B"])
        .with_parent("Child", "Other");
    let arena = lookup.type_arena().unwrap();
    let member = sym(1, "m", "Base.m", "method", "a.ts");

    let out = substitute_supertype_args(
        &lookup,
        arena,
        &member,
        arena.class("B"),
        arena.class("Child"),
        None,
    );

    assert_eq!(arena.get(out), Type::Class("B".to_string()));
}

#[test]
fn a_supertype_cycle_terminates() {
    let lookup = Lookup::new()
        .with_generics("Base", &["B"])
        .with_parent("A", "B")
        .with_parent_args("A", "B", &["User"])
        .with_parent("B", "A")
        .with_parent_args("B", "A", &["User"]);
    let arena = lookup.type_arena().unwrap();
    let member = sym(1, "m", "Base.m", "method", "a.ts");

    let out = substitute_supertype_args(
        &lookup,
        arena,
        &member,
        arena.class("B"),
        arena.class("A"),
        None,
    );

    assert_eq!(arena.get(out), Type::Class("B".to_string()));
}
