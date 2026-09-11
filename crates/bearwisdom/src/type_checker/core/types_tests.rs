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
fn rebind_canonicalizes_higher_kinded_base() {
    use rustc_hash::FxHashMap;
    // `F<A>` interned nominally rebinds BOTH the base and the arg to their
    // canonical generic params, so a higher-kinded return type substitutes.
    let mut arena = TypeArena::new();
    let f_param = arena.intern_generic(GenericParamData {
        kind: Default::default(),
        name: "F".into(),
        owner_symbol_index: 0,
        bound: None,
    });
    let a_param = arena.intern_generic(GenericParamData {
        kind: Default::default(),
        name: "A".into(),
        owner_symbol_index: 0,
        bound: None,
    });
    let gen_f = arena.intern(Type::Generic { param: f_param });
    let gen_a = arena.intern(Type::Generic { param: a_param });
    let nominal_f = arena.class("F");
    let nominal_a = arena.class("A");
    let nominal = arena.intern(Type::Apply {
        base: nominal_f,
        args: vec![nominal_a],
    });
    let mut map = FxHashMap::default();
    map.insert("F".to_string(), gen_f);
    map.insert("A".to_string(), gen_a);
    let out = arena.rebind_class_params(nominal, &map);
    assert_eq!(
        arena.get(out),
        Type::Apply {
            base: gen_f,
            args: vec![gen_a]
        }
    );
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
        kind: Default::default(),
        name: "T".to_string(),
        owner_symbol_index: 5,
        bound: None,
    });
    let u = arena.intern_generic(GenericParamData {
        kind: Default::default(),
        name: "U".to_string(),
        owner_symbol_index: 5,
        bound: None,
    });
    assert_ne!(t, u);
    assert_eq!(arena.generic_param(t).name, "T");
    assert_eq!(arena.generic_param(u).owner_symbol_index, 5);
}

#[test]
fn snapshot_round_trips_every_variant_preserving_ids() {
    let a = TypeArena::new();
    let user = a.class("User");
    let vec_user = a.intern(Type::Apply {
        base: a.class("Vec"),
        args: vec![user],
    });
    let opt = a.intern(Type::Optional(vec_user));
    let _prim = a.primitive(PrimKind::Int);
    let gp = a.intern_generic(GenericParamData {
        kind: Default::default(),
        name: "T".to_string(),
        owner_symbol_index: 3,
        bound: Some(user),
    });
    let gen = a.intern(Type::Generic { param: gp });
    let lit = a.intern(Type::Literal(LitValue::Str("x".to_string())));

    let blob = a.serialize_snapshot();
    let b = TypeArena::new();
    let n = b.restore_snapshot(&blob);

    assert!(n >= 6, "all interned types restored");
    // TypeIds are preserved verbatim, so a persisted raw id is still valid.
    assert_eq!(b.get(opt), a.get(opt));
    assert_eq!(b.get(gen), a.get(gen));
    assert_eq!(b.get(lit), a.get(lit));
    // qname index rebuilt → class() dedups to the restored id.
    assert_eq!(b.class_lookup("User"), Some(user));
    // generic params restored (bound included).
    assert_eq!(b.generic_param(gp).name, "T");
    assert_eq!(b.generic_param(gp).bound, Some(user));
    // intern dedups against the restored set rather than minting a new id.
    assert_eq!(b.intern(Type::Optional(vec_user)), opt);
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

#[test]
fn format_type_renders_class() {
    let arena = TypeArena::new();
    let id = arena.class("User");
    assert_eq!(arena.format_type(id), "User");
}

#[test]
fn format_type_renders_apply_with_one_arg() {
    let arena = TypeArena::new();
    let base = arena.class("Repository");
    let user = arena.class("User");
    let id = arena.intern(Type::Apply {
        base,
        args: vec![user],
    });
    assert_eq!(arena.format_type(id), "Repository<User>");
}

#[test]
fn format_type_renders_apply_with_multiple_args() {
    let arena = TypeArena::new();
    let base = arena.class("Map");
    let key = arena.class("K");
    let value = arena.class("V");
    let id = arena.intern(Type::Apply {
        base,
        args: vec![key, value],
    });
    assert_eq!(arena.format_type(id), "Map<K, V>");
}

#[test]
fn format_type_round_trips_nested_generic() {
    let arena = TypeArena::new();
    let result_base = arena.class("Result");
    let ok = arena.class("Ok");
    let err = arena.class("Err");
    let result = arena.intern(Type::Apply {
        base: result_base,
        args: vec![ok, err],
    });
    let promise = arena.class("Promise");
    let id = arena.intern(Type::Apply {
        base: promise,
        args: vec![result],
    });
    assert_eq!(arena.format_type(id), "Promise<Result<Ok, Err>>");
}

// ---------------------------------------------------------------------------
// Type::Decl — per-declaration nominal identity
// ---------------------------------------------------------------------------

#[test]
fn decl_interns_per_declaration_not_per_qname() {
    let arena = TypeArena::new();
    let a = arena.decl("Foo", 41);
    let b = arena.decl("Foo", 42);
    let c = arena.decl("Foo", 41);
    assert_ne!(a, b, "same qname, different declarations: distinct TypeIds");
    assert_eq!(a, c, "same declaration interns once");
    // The name-addressed arm remains a distinct concept from either binding.
    assert_ne!(arena.class("Foo"), a);
    assert_ne!(arena.class("Foo"), b);
}

#[test]
fn declaration_intern_and_lookup_ignore_display_spelling() {
    let arena = TypeArena::new();
    let original = arena.decl("Original", 73);
    let mut renamed = arena.get(original);
    if let Type::Decl { qname, .. } = &mut renamed {
        *qname = "poisoned display".into();
    }
    assert_eq!(arena.lookup(&renamed), Some(original));
    assert_eq!(arena.intern(renamed), original);
}

#[test]
fn restore_rebuilds_declaration_index_without_reusing_previous_rows() {
    let source = TypeArena::new();
    let expected = source.decl("Snapshot", 73);
    let destination = TypeArena::new();
    destination.class("PreviousFirstSlot");
    let obsolete = destination.decl("Previous", 73);
    assert_ne!(obsolete, expected);
    destination.restore_snapshot(&source.serialize_snapshot());
    assert_eq!(destination.decl("new display", 73), expected);
    assert!(matches!(
        destination.get(expected),
        Type::Decl { symbol_id: 73, .. }
    ));
}

#[test]
fn decl_formats_as_its_qname() {
    let arena = TypeArena::new();
    let d = arena.decl("Ns.Repo", 7);
    assert_eq!(arena.format_type(d), "Ns.Repo");
    let applied = arena.intern(Type::Apply {
        base: d,
        args: vec![arena.class("User")],
    });
    assert_eq!(arena.format_type(applied), "Ns.Repo<User>");
}

#[test]
fn rebind_class_params_never_touches_a_decl() {
    let arena = TypeArena::new();
    let d = arena.decl("T", 9);
    let mut params = FxHashMap::default();
    let gp = arena.intern_generic(GenericParamData {
        kind: Default::default(),
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    params.insert("T".to_string(), arena.intern(Type::Generic { param: gp }));
    assert_eq!(
        arena.rebind_class_params(d, &params),
        d,
        "a bound nominal is not a param name"
    );
}
