use super::*;
use crate::indexer::resolve::engine::testkit::{sym, Lookup};
use crate::type_checker::core::types::Type;
use crate::types::AliasTarget;

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
fn expand_with_id_prefers_id_keyed_target_over_colliding_bare_name() {
    // Two `Logger` aliases collide on the bare name: the name map holds the WRONG
    // sibling (a dead `WrongRet`), the id map holds the right one (decl id 8534):
    // type Logger = ReturnType<typeof createScopedLogger>; createScopedLogger(): ScopedRet.
    let lookup = Lookup::new()
        .with_alias("Logger", app("WrongRet", &[]))
        .with_alias_id(
            8534,
            AliasTarget::Application {
                root: "ReturnType".to_string(),
                args: vec!["createScopedLogger".to_string()],
            },
        )
        .with(sym(1, "createScopedLogger", "createScopedLogger", "function", "a.ts"))
        .with_return_type("createScopedLogger", "ScopedRet");
    let arena = lookup.type_arena().unwrap();
    // With the use-site id, the alias resolves to ITS target — createScopedLogger's return.
    let out = expand_with_id(arena.class("Logger"), Some(8534), &lookup, arena);
    assert_eq!(arena.format_type(out), "ScopedRet");
    // Without the id, the name map's (wrong) last-writer target wins.
    let bare = expand(arena.class("Logger"), &lookup, arena);
    assert_eq!(arena.format_type(bare), "WrongRet");
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

#[test]
fn evaluates_a_decidable_conditional_to_its_false_branch() {
    // type Upd<T, D, K> = D extends true ? T : Omit<Base, K>
    // Upd<Base, false, "x"> — `false extends true` is decidably false, so the
    // false branch Omit<Base,"x"> is taken, then the direct member-preserving
    // Omit unwraps to Base (the query-builder `.set()` shape: a `TDynamic` toggle
    // whose false arm is the concrete builder).
    let lookup = Lookup::new()
        .with_alias(
            "Upd",
            AliasTarget::Conditional {
                check: "D".to_string(),
                extends: "true".to_string(),
                true_branch: "T".to_string(),
                false_branch: "Omit<Base, K>".to_string(),
                infer_binding: None,
            },
        )
        .with_generics("Upd", &["T", "D", "K"]);
    let arena = lookup.type_arena().unwrap();
    let applied = arena.intern(Type::Apply {
        base: arena.class("Upd"),
        args: vec![arena.class("Base"), arena.class("false"), arena.class("x")],
    });
    assert_eq!(arena.format_type(expand(applied, &lookup, arena)), "Base");
}

#[test]
fn leaves_an_undecidable_conditional_unevaluated() {
    // type Cond<T> = T extends string ? A : B — `T extends string` is undecidable
    // without a subtype lattice, so neither branch is guessed; with no recorded
    // field type the type stays put.
    let lookup = Lookup::new()
        .with_alias(
            "Cond",
            AliasTarget::Conditional {
                check: "T".to_string(),
                extends: "string".to_string(),
                true_branch: "A".to_string(),
                false_branch: "B".to_string(),
                infer_binding: None,
            },
        )
        .with_generics("Cond", &["T"]);
    let arena = lookup.type_arena().unwrap();
    let applied = arena.intern(Type::Apply {
        base: arena.class("Cond"),
        args: vec![arena.class("User")],
    });
    assert_eq!(arena.format_type(expand(applied, &lookup, arena)), "Cond<User>");
}
