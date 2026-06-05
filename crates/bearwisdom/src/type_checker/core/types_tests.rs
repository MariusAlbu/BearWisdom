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
fn intern_type_str_parses_function_type() {
    let mut arena = TypeArena::new();
    let f = arena.intern_type_str("() => User");
    match arena.get(f) {
        Type::Function { params, return_ } => {
            assert!(params.is_empty());
            assert!(matches!(arena.get(return_), Type::Class(q) if q == "User"));
        }
        other => panic!("expected Function, got {other:?}"),
    }
    // Params with annotations and a generic return are still a function type.
    assert!(matches!(
        arena.get(arena.intern_type_str("(x: number) => Box<User>")),
        Type::Function { .. }
    ));
    // A generic that merely carries a function-typed arg must NOT be read as a
    // function type — the arrow is nested, not top-level.
    assert!(!matches!(
        arena.get(arena.intern_type_str("Foo<() => void>")),
        Type::Function { .. }
    ));
    // A plain nominal type is unaffected.
    assert!(matches!(arena.get(arena.intern_type_str("User")), Type::Class(_)));
}

#[test]
fn intern_type_str_parses_arrow_return_function_type() {
    let mut arena = TypeArena::new();
    // Rust `Fn() -> T`, Kotlin / Swift `(T) -> R`, and `impl Fn(..) -> T` all
    // carry a top-level `->` whose right side is the return type.
    for s in ["Fn() -> User", "(x: I32) -> User", "impl Fn() -> User"] {
        match arena.get(arena.intern_type_str(s)) {
            Type::Function { return_, .. } => {
                assert!(
                    matches!(arena.get(return_), Type::Class(q) if q == "User"),
                    "{s} should yield User"
                );
            }
            other => panic!("expected Function for {s}, got {other:?}"),
        }
    }
    // A nested `->` inside generic brackets is not a top-level function type.
    assert!(!matches!(
        arena.get(arena.intern_type_str("Box<dyn Fn() -> User>")),
        Type::Function { .. }
    ));
}

#[test]
fn intern_type_str_preserves_function_param_types() {
    let mut arena = TypeArena::new();
    // TS `(name: T) =>` — the param annotation is peeled to its bare type, so
    // the param interns as `Class("T")` (rebind lifts Class→Generic later).
    let t = arena.intern_type_str("T");
    let f = arena.intern_type_str("(value: T) => U");
    match arena.get(f) {
        Type::Function { params, return_ } => {
            assert_eq!(params, vec![t]);
            assert!(matches!(arena.get(return_), Type::Class(q) if q == "U"));
        }
        other => panic!("expected Function, got {other:?}"),
    }
    // Rust `Fn(T) -> U` — the param is bare (no colon), so the whole piece is
    // the type. The param list is the first top-level parens after `Fn`.
    match arena.get(arena.intern_type_str("Fn(T) -> U")) {
        Type::Function { params, return_ } => {
            assert_eq!(params, vec![arena.intern_type_str("T")]);
            assert!(matches!(arena.get(return_), Type::Class(q) if q == "U"));
        }
        other => panic!("expected Function, got {other:?}"),
    }
}

#[test]
fn rebind_canonicalizes_higher_kinded_base() {
    use rustc_hash::FxHashMap;
    // `F<A>` interned nominally rebinds BOTH the base and the arg to their
    // canonical generic params, so a higher-kinded return type substitutes.
    let mut arena = TypeArena::new();
    let f_param = arena.intern_generic(GenericParamData {
        name: "F".into(),
        owner_symbol_index: 0,
        bound: None,
    });
    let a_param = arena.intern_generic(GenericParamData {
        name: "A".into(),
        owner_symbol_index: 0,
        bound: None,
    });
    let gen_f = arena.intern(Type::Generic { param: f_param });
    let gen_a = arena.intern(Type::Generic { param: a_param });
    let nominal = arena.intern_type_str("F<A>");
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

#[test]
fn intern_type_str_handles_simple_class() {
    let arena = TypeArena::new();
    let id = arena.intern_type_str("User");
    match arena.get(id) {
        Type::Class(q) => assert_eq!(q, "User"),
        other => panic!("expected Class, got {other:?}"),
    }
}

#[test]
fn intern_type_str_trims_whitespace() {
    let arena = TypeArena::new();
    let trimmed = arena.intern_type_str("  Foo  ");
    let direct = arena.class("Foo");
    assert_eq!(trimmed, direct);
}

#[test]
fn intern_type_str_decomposes_generic_application() {
    let arena = TypeArena::new();
    let id = arena.intern_type_str("Repository<User>");
    match arena.get(id) {
        Type::Apply { base, args } => {
            assert_eq!(args.len(), 1);
            match arena.get(base) {
                Type::Class(q) => assert_eq!(q, "Repository"),
                other => panic!("expected Class base, got {other:?}"),
            }
            match arena.get(args[0]) {
                Type::Class(q) => assert_eq!(q, "User"),
                other => panic!("expected Class arg, got {other:?}"),
            }
        }
        other => panic!("expected Apply, got {other:?}"),
    }
}

#[test]
fn intern_type_str_decomposes_multi_arg_generic() {
    let arena = TypeArena::new();
    let id = arena.intern_type_str("Map<K, V>");
    let Type::Apply { base, args } = arena.get(id) else {
        panic!("expected Apply");
    };
    assert_eq!(arena.get(base), Type::Class("Map".to_string()));
    assert_eq!(args.len(), 2);
    assert_eq!(arena.get(args[0]), Type::Class("K".to_string()));
    assert_eq!(arena.get(args[1]), Type::Class("V".to_string()));
}

#[test]
fn intern_type_str_handles_nested_generics() {
    let arena = TypeArena::new();
    let id = arena.intern_type_str("Promise<Result<Ok, Err>>");
    let Type::Apply { base, args } = arena.get(id) else {
        panic!("expected outer Apply");
    };
    assert_eq!(arena.get(base), Type::Class("Promise".to_string()));
    assert_eq!(args.len(), 1);
    let Type::Apply { base: inner_base, args: inner_args } = arena.get(args[0]) else {
        panic!("expected inner Apply");
    };
    assert_eq!(arena.get(inner_base), Type::Class("Result".to_string()));
    assert_eq!(inner_args.len(), 2);
    assert_eq!(arena.get(inner_args[0]), Type::Class("Ok".to_string()));
    assert_eq!(arena.get(inner_args[1]), Type::Class("Err".to_string()));
}

#[test]
fn intern_type_str_dedups_identical_apply() {
    let arena = TypeArena::new();
    let a = arena.intern_type_str("Repository<User>");
    let b = arena.intern_type_str("Repository<User>");
    assert_eq!(a, b);
}

#[test]
fn intern_type_str_accepts_scala_bracket_style() {
    let arena = TypeArena::new();
    let id = arena.intern_type_str("Map[K, V]");
    let Type::Apply { base, args } = arena.get(id) else {
        panic!("expected Apply for Scala-style brackets");
    };
    assert_eq!(arena.get(base), Type::Class("Map".to_string()));
    assert_eq!(args.len(), 2);
}

#[test]
fn intern_type_str_falls_back_to_class_for_unbalanced() {
    let arena = TypeArena::new();
    let id = arena.intern_type_str("Foo<Bar");
    // Unbalanced — entire string becomes a Class.
    assert_eq!(arena.get(id), Type::Class("Foo<Bar".to_string()));
}

#[test]
fn intern_type_str_falls_back_for_anonymous_generic() {
    let arena = TypeArena::new();
    let id = arena.intern_type_str("<Bar>");
    // Empty head — fallback to Class on the raw input.
    assert_eq!(arena.get(id), Type::Class("<Bar>".to_string()));
}

#[test]
fn intern_type_str_falls_back_on_post_bracket_text() {
    let arena = TypeArena::new();
    let id = arena.intern_type_str("Foo<Bar>.Baz");
    // Anything after the closing bracket isn't first-class — fallback.
    assert_eq!(arena.get(id), Type::Class("Foo<Bar>.Baz".to_string()));
}

#[test]
fn intern_type_str_empty_string_falls_back_to_class() {
    let arena = TypeArena::new();
    let id = arena.intern_type_str("");
    assert_eq!(arena.get(id), Type::Class("".to_string()));
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
    let id = arena.intern_type_str("Repository<User>");
    assert_eq!(arena.format_type(id), "Repository<User>");
}

#[test]
fn format_type_renders_apply_with_multiple_args() {
    let arena = TypeArena::new();
    let id = arena.intern_type_str("Map<K, V>");
    assert_eq!(arena.format_type(id), "Map<K, V>");
}

#[test]
fn format_type_round_trips_nested_generic() {
    let arena = TypeArena::new();
    let id = arena.intern_type_str("Promise<Result<Ok, Err>>");
    assert_eq!(arena.format_type(id), "Promise<Result<Ok, Err>>");
}

#[test]
fn format_type_round_trips_through_arena() {
    let arena = TypeArena::new();
    let original = "Outer<Middle<Inner>>";
    let id = arena.intern_type_str(original);
    let formatted = arena.format_type(id);
    let id2 = arena.intern_type_str(&formatted);
    assert_eq!(id, id2);
}

#[test]
fn intern_type_str_strips_reference_sigil() {
    let arena = TypeArena::new();
    // A leading `&` (with an optional lifetime and `mut`) is a reference sigil,
    // never part of a class name — strip it so the receiver interns as the
    // referent, matching the type a `&C` / `&mut C` binding members against.
    let c = arena.class("C");
    assert_eq!(arena.intern_type_str("&C"), c);
    assert_eq!(arena.intern_type_str("&mut C"), c);
    assert_eq!(arena.intern_type_str("&'a C"), c);
    assert_eq!(arena.intern_type_str("& 'a mut C"), c);
    // A reference to a generic application strips the sigil and keeps the Apply.
    assert_eq!(arena.intern_type_str("&Box<C>"), arena.intern_type_str("Box<C>"));
}

#[test]
fn intern_type_str_drops_leading_lifetime_arg() {
    let arena = TypeArena::new();
    // A generic argument that is a lifetime (`'a`, `'static`) is never a type —
    // it is dropped from the Apply's args so a wrapper whose first param is a
    // lifetime (`Cow<'a, str>`) interns as a SINGLE-type-arg application, the
    // shape the single-inner-wrapper peel projects through.
    let str_ty = arena.class("str");
    let cow_str = arena.intern_type_str("Cow<'a, str>");
    match arena.get(cow_str) {
        Type::Apply { base, args } => {
            assert!(matches!(arena.get(base), Type::Class(q) if q == "Cow"));
            assert_eq!(args, vec![str_ty], "lifetime arg dropped, only `str` remains");
        }
        other => panic!("expected Apply, got {other:?}"),
    }
    // A `'static` lifetime is dropped the same way.
    let cow_static = arena.intern_type_str("Cow<'static, str>");
    assert_eq!(cow_static, cow_str, "any lifetime arg is dropped identically");
    // An all-lifetime arg list collapses to the bare base class (no type args).
    let only_life = arena.intern_type_str("Ref<'a>");
    assert!(matches!(arena.get(only_life), Type::Class(q) if q == "Ref"));
}

#[test]
fn intern_type_str_leaves_interior_ampersand_alone() {
    let arena = TypeArena::new();
    // An interior `&` is a TS intersection operator, not a reference sigil —
    // only a byte-0 `&` is stripped, so `A & B` still falls through to the
    // class fallback unchanged.
    let id = arena.intern_type_str("A & B");
    assert!(matches!(arena.get(id), Type::Class(q) if q == "A & B"));
}

#[test]
fn intern_type_str_strips_opaque_existential_prefix() {
    let arena = TypeArena::new();
    // A leading `some `/`any ` is Swift's opaque/existential keyword prefix,
    // never part of a class name — strip it so a value typed `some Greet` /
    // `any Greet` interns as the bare protocol `Greet`, the base under which a
    // protocol extension files its default members.
    let greet = arena.class("Greet");
    assert_eq!(arena.intern_type_str("some Greet"), greet);
    assert_eq!(arena.intern_type_str("any Greet"), greet);
    // Extra interior whitespace is tolerated (the prefix is keyword + space).
    assert_eq!(arena.intern_type_str("some   Greet"), greet);
    // A constrained existential keeps its application: `any Collection<Int>`
    // strips the keyword and re-interns the constrained type.
    assert_eq!(
        arena.intern_type_str("any Collection<Int>"),
        arena.intern_type_str("Collection<Int>")
    );
}

#[test]
fn intern_type_str_does_not_strip_some_any_as_type_name_prefix() {
    let arena = TypeArena::new();
    // `some`/`any` are only stripped as standalone keyword prefixes (followed by
    // whitespace then a type). A class whose NAME begins with those letters but
    // is not the bare keyword (`Something`, `anyOf`, `SomeType`) is untouched —
    // no word boundary, no strip.
    assert!(matches!(arena.get(arena.intern_type_str("Something")), Type::Class(q) if q == "Something"));
    assert!(matches!(arena.get(arena.intern_type_str("anyOf")), Type::Class(q) if q == "anyOf"));
    assert!(matches!(arena.get(arena.intern_type_str("SomeType")), Type::Class(q) if q == "SomeType"));
}
