use super::*;
use crate::indexer::resolve::engine::testkit::Lookup;
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
