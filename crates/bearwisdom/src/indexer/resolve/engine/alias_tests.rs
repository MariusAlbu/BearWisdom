use super::*;
use crate::indexer::resolve::engine::testkit::{sym, Lookup};
use crate::type_checker::core::types::Type;

fn app(root: &str, args: &[&str]) -> AliasTarget {
    AliasTarget::Application {
        root: root.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn expands_non_generic_alias_to_its_target() {
    // type UserMap = Map<string, User>
    let lookup = Lookup::new().with_alias("UserMap", app("Map", &["string", "User"]));
    let arena = lookup.type_arena().unwrap();
    let out = expand(arena.class("UserMap"), &lookup, arena);
    assert_eq!(arena.format_type(out), "Map<string, User>");
}

#[test]
fn expands_generic_alias_substituting_its_parameter() {
    // type Box<T> = Container<T>;  Box<User> → Container<User>
    let lookup = Lookup::new()
        .with_alias("Box", app("Container", &["T"]))
        .with_generics("Box", &["T"]);
    let arena = lookup.type_arena().unwrap();
    let boxed = arena.intern(Type::Apply {
        base: arena.class("Box"),
        args: vec![arena.class("User")],
    });
    let out = expand(boxed, &lookup, arena);
    assert_eq!(arena.format_type(out), "Container<User>");
}

#[test]
fn leaves_a_non_alias_unchanged() {
    let lookup = Lookup::new();
    let arena = lookup.type_arena().unwrap();
    let t = arena.class("User");
    assert_eq!(expand(t, &lookup, arena), t);
}

#[test]
fn follows_an_alias_of_an_alias() {
    // type A = B;  type B = Map<K, V>   →   A expands through to Map
    let lookup = Lookup::new()
        .with_alias("A", app("B", &[]))
        .with_alias("B", app("Map", &["K", "V"]));
    let arena = lookup.type_arena().unwrap();
    let out = expand(arena.class("A"), &lookup, arena);
    assert_eq!(arena.format_type(out), "Map<K, V>");
}

#[test]
fn follows_a_member_less_alias_through_its_field_type() {
    // type ExpectTypeOf<T> = … ? PositiveExpectTypeOf<T> : Negative<T> — a
    // conditional alias with no members of its own, transparent through its
    // flattened RHS head so member lookup finds the underlying type's members.
    let lookup = Lookup::new()
        .with(sym(1, "ExpectTypeOf", "ExpectTypeOf", "type_alias", "f.ts"))
        .with_field_type("ExpectTypeOf", "PositiveExpectTypeOf");
    let arena = lookup.type_arena().unwrap();
    let out = expand(arena.class("ExpectTypeOf"), &lookup, arena);
    assert_eq!(arena.format_type(out), "PositiveExpectTypeOf");
}

#[test]
fn keeps_an_object_literal_alias_that_carries_its_own_members() {
    // type Foo = Bar & { x: number } — declares member `x`; following its field
    // type to `Bar` would drop `x`, so a member-bearing alias stays in place.
    let lookup = Lookup::new()
        .with(sym(1, "Foo", "Foo", "type_alias", "f.ts"))
        .with_field_type("Foo", "Bar")
        .with_member_id(1, sym(2, "x", "Foo.x", "property", "f.ts"));
    let arena = lookup.type_arena().unwrap();
    let foo = arena.class("Foo");
    assert_eq!(expand(foo, &lookup, arena), foo);
}
