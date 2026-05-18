use super::*;

#[test]
fn intern_dedups_identical_types() {
    let mut arena = TypeArena::new();
    let a = arena.intern(Type::Primitive(PrimKind::Int));
    let b = arena.intern(Type::Primitive(PrimKind::Int));
    assert_eq!(a, b);
    assert_eq!(arena.len(), 1);
}

#[test]
fn distinct_types_produce_distinct_ids() {
    let mut arena = TypeArena::new();
    let int_id = arena.intern(Type::Primitive(PrimKind::Int));
    let str_id = arena.intern(Type::Primitive(PrimKind::Str));
    assert_ne!(int_id, str_id);
    assert_eq!(arena.len(), 2);
}

#[test]
fn class_returns_stable_id_per_qname() {
    let mut arena = TypeArena::new();
    let a = arena.class("com.foo.User");
    let b = arena.class("com.foo.User");
    let c = arena.class("com.foo.Other");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn class_lookup_finds_interned_class_without_inserting() {
    let mut arena = TypeArena::new();
    assert!(arena.class_lookup("missing").is_none());
    let id = arena.class("present");
    assert_eq!(arena.class_lookup("present"), Some(id));
}

#[test]
fn get_resolves_typeid_to_type() {
    let mut arena = TypeArena::new();
    let id = arena.intern(Type::Class("X".to_string()));
    match arena.get(id) {
        Type::Class(q) => assert_eq!(q, "X"),
        other => panic!("expected Class, got {other:?}"),
    }
}

#[test]
fn nested_apply_interns_each_level() {
    let mut arena = TypeArena::new();
    let user = arena.class("User");
    let list = arena.class("List");
    let list_of_user = arena.intern(Type::Apply {
        base: list,
        args: vec![user],
    });
    assert_ne!(list_of_user, list);
    assert_ne!(list_of_user, user);
    let again = arena.intern(Type::Apply {
        base: list,
        args: vec![user],
    });
    assert_eq!(list_of_user, again);
}

#[test]
fn function_type_intern_dedups_on_identical_signatures() {
    let mut arena = TypeArena::new();
    let int = arena.primitive(PrimKind::Int);
    let bool_ = arena.primitive(PrimKind::Bool);
    let fn1 = arena.intern(Type::Function {
        params: vec![int],
        return_: bool_,
    });
    let fn2 = arena.intern(Type::Function {
        params: vec![int],
        return_: bool_,
    });
    assert_eq!(fn1, fn2);
}

#[test]
fn generic_param_allocation_yields_unique_ids() {
    let mut arena = TypeArena::new();
    let t = arena.intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 5,
        bound: None,
    });
    let u = arena.intern_generic(GenericParamData {
        name: "U".to_string(),
        owner_symbol_index: 5,
        bound: None,
    });
    assert_ne!(t, u);
    assert_eq!(arena.generic_param(t).name, "T");
    assert_eq!(arena.generic_param(u).owner_symbol_index, 5);
}

#[test]
fn typeid_index_round_trips_via_get() {
    let mut arena = TypeArena::new();
    let a = arena.intern(Type::Primitive(PrimKind::Bool));
    let b = arena.intern(Type::Primitive(PrimKind::Float));
    assert_eq!(a.index(), 0);
    assert_eq!(b.index(), 1);
}

#[test]
fn lookup_returns_none_for_uninterned_type() {
    let arena = TypeArena::new();
    assert!(arena.lookup(&Type::Primitive(PrimKind::Int)).is_none());
}
