use super::*;
use crate::indexer::resolve::engine::testkit::{sym, Lookup};
use crate::type_checker::core::types::TypeArena;

#[test]
fn a_value_carrying_the_head_qname_is_not_the_receiver_declaration() {
    // A property flattened out of an anonymous object arm can land on a bare
    // qname (`any`, `all`, `path`), which then collides with every receiver
    // whose head is that name. A value is never a receiver declaration.
    let lookup = Lookup::new().with(sym(1, "any", "any", "property", "ext:ts:pkg/config.d.ts"));
    let arena = TypeArena::new();
    let ty = arena.class("any");
    assert_eq!(head_symbol_id(&arena, &lookup, ty, None), None);
    assert_eq!(head_symbol_id_preferring_package(&arena, &lookup, ty, None), None);
}

#[test]
fn a_type_carrying_the_head_qname_still_binds() {
    let lookup = Lookup::new().with(sym(1, "User", "app.User", "interface", "src/user.ts"));
    let arena = TypeArena::new();
    let ty = arena.class("app.User");
    assert_eq!(head_symbol_id(&arena, &lookup, ty, None), Some(1));
    assert_eq!(head_symbol_id_preferring_package(&arena, &lookup, ty, None), Some(1));
}

#[test]
fn a_type_wins_over_a_value_sharing_one_qname() {
    // The merged value+type pair (`declare var D: DConstructor` alongside
    // `interface D`) puts both under one qname; the receiver is the type.
    let lookup = Lookup::new()
        .with(sym(1, "D", "D", "variable", "ext:ts:__ts_lib__/lib.dom.d.ts"))
        .with(sym(2, "D", "D", "interface", "ext:ts:__ts_lib__/lib.dom.d.ts"));
    let arena = TypeArena::new();
    assert_eq!(head_symbol_id(&arena, &lookup, arena.class("D"), None), Some(2));
}

#[test]
fn the_package_preferred_pick_also_requires_a_type() {
    // A same-qname value inside the preferred package must not out-rank the
    // type declaration the head actually names.
    let lookup = Lookup::new()
        .with_in_package(7, sym(1, "Item", "pkg.Item", "property", "a.ts"))
        .with(sym(2, "Item", "pkg.Item", "class", "b.ts"));
    let arena = TypeArena::new();
    let ty = arena.class("pkg.Item");
    assert_eq!(
        head_symbol_id_preferring_package(&arena, &lookup, ty, Some(7)),
        Some(2)
    );
}

#[test]
fn a_head_naming_nothing_stays_unbound() {
    let lookup = Lookup::new();
    let arena = TypeArena::new();
    assert_eq!(head_symbol_id(&arena, &lookup, arena.class("Missing"), None), None);
}

#[test]
fn head_decl_id_carries_through_wrappers_and_apply() {
    use crate::type_checker::core::types::{Type, TypeArena};
    let arena = TypeArena::new();
    let d = arena.decl("Repo", 42);
    assert_eq!(super::head_decl_id(&arena, d), Some(42));
    let applied = arena.intern(Type::Apply { base: d, args: vec![arena.class("User")] });
    assert_eq!(super::head_decl_id(&arena, applied), Some(42));
    let opt = arena.intern(Type::Optional(applied));
    assert_eq!(super::head_decl_id(&arena, opt), Some(42));
    assert_eq!(super::head_decl_id(&arena, arena.class("Repo")), None, "name-only heads carry nothing");
    // The string bridge: name-keyed consumers see a Decl exactly like a Class.
    assert_eq!(super::head_qname(&arena, applied).as_deref(), Some("Repo"));
}
