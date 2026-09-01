use rustc_hash::FxHashMap;

use super::apply;
use crate::indexer::resolve::engine::contract::TypeInfo;
use crate::indexer::resolve::engine::contract::SymbolLookup;
use crate::indexer::resolve::engine::head_decl::head_decl_id;
use crate::indexer::resolve::engine::testkit::{sym, Lookup};
use crate::type_checker::core::types::Type;

#[test]
fn unique_head_binds_and_survives_wrappers() {
    let lookup = Lookup::new().with(sym(7, "User", "pkg.User", "class", "a.ts"));
    let arena = lookup.type_arena().unwrap();
    let stored = arena.intern(Type::Optional(arena.intern(Type::Apply {
        base: arena.class("pkg.User"),
        args: vec![arena.class("T")],
    })));
    let mut slots: FxHashMap<i64, TypeInfo> = FxHashMap::default();
    slots.insert(
        1,
        TypeInfo {
            return_type_id: Some(stored),
            ..Default::default()
        },
    );

    apply(arena, &lookup, &mut slots);

    let bound = slots.get(&1).unwrap().return_type_id.unwrap();
    assert_eq!(head_decl_id(arena, bound), Some(7), "head bound through Optional<Apply<..>>");
}

#[test]
fn ambiguous_head_stays_name_addressed() {
    // Two type declarations share the qname (cross-package duplicate) — the
    // stored head must NOT bind to either; read-time recovery owns the pick.
    let lookup = Lookup::new()
        .with(sym(7, "Page", "ui.Page", "class", "a.ts"))
        .with(sym(8, "Page", "ui.Page", "class", "other/b.ts"));
    let arena = lookup.type_arena().unwrap();
    let stored = arena.class("ui.Page");
    let mut slots: FxHashMap<i64, TypeInfo> = FxHashMap::default();
    slots.insert(1, TypeInfo { field_type_id: Some(stored), ..Default::default() });

    apply(arena, &lookup, &mut slots);

    let kept = slots.get(&1).unwrap().field_type_id.unwrap();
    assert_eq!(head_decl_id(arena, kept), None, "ambiguous heads keep Class form");
}


#[test]
fn bare_head_never_binds_even_when_unique() {
    // `find(): T` stores Class("T"); a same-named declaration anywhere must
    // not capture it — T is the owner's generic parameter, and only
    // rebind_class_params may rewrite it.
    let lookup = Lookup::new().with(sym(7, "T", "T", "class", "fixture.ts"));
    let arena = lookup.type_arena().unwrap();
    let stored = arena.class("T");
    let mut slots: FxHashMap<i64, TypeInfo> = FxHashMap::default();
    slots.insert(1, TypeInfo { return_type_id: Some(stored), ..Default::default() });

    apply(arena, &lookup, &mut slots);

    let kept = slots.get(&1).unwrap().return_type_id.unwrap();
    assert_eq!(head_decl_id(arena, kept), None, "bare heads keep Class form");
}
