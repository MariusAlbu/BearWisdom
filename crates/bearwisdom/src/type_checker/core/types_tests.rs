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
    assert!(matches!(
        arena.get(arena.intern_type_str("User")),
        Type::Class(_)
    ));
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
fn intern_type_str_parses_fixed_arity_python_callable_annotations() {
    let mut arena = TypeArena::new();
    let a = arena.intern_type_str("A");
    let b = arena.intern_type_str("B");
    let r = arena.intern_type_str("R");
    for source in [
        "Callable[[A, B], R]",
        "typing.Callable[[A, B], R]",
        "collections.abc.Callable[[A, B], R]",
    ] {
        match arena.get(arena.intern_type_str(source)) {
            Type::Function { params, return_ } => {
                assert_eq!(params, vec![a, b], "{source}");
                assert_eq!(return_, r, "{source}");
            }
            other => panic!("expected Python Callable Function for {source}, got {other:?}"),
        }
    }

    match arena
        .get(arena.intern_type_str("typing.Callable[[Callable[[A], B], list[Item]], Result]"))
    {
        Type::Function { params, return_ } => {
            assert_eq!(params.len(), 2);
            assert!(matches!(arena.get(params[0]), Type::Function { .. }));
            assert!(matches!(arena.get(params[1]), Type::Apply { .. }));
            assert!(matches!(arena.get(return_), Type::Class(name) if name == "Result"));
        }
        other => panic!("expected nested Python Callable Function, got {other:?}"),
    }

    match arena.get(arena.intern_type_str("collections.abc.Callable[[], None]")) {
        Type::Function { params, return_ } => {
            assert!(params.is_empty());
            assert!(matches!(arena.get(return_), Type::Class(name) if name == "None"));
        }
        other => panic!("expected zero-argument Python Callable Function, got {other:?}"),
    }
}

#[test]
fn intern_type_str_keeps_unmodeled_python_callable_forms_opaque() {
    let mut arena = TypeArena::new();
    for source in [
        "Callable",
        "CallbackAlias[[A], R]",
        "typing_extensions.Callable[[A], R]",
        "Callable[..., R]",
        "Callable[P, R]",
        "Callable[Concatenate[A, P], R]",
        "Callable[[P.args], R]",
        "Callable[[ParamSpec], R]",
        "Callable[[~T], ~T]",
        "Callable[[A], R, Extra]",
    ] {
        assert!(
            !matches!(
                arena.get(arena.intern_type_str(source)),
                Type::Function { .. }
            ),
            "{source} must stay opaque without callable binder identity"
        );
    }
}

#[test]
fn intern_type_str_parses_dart_return_first_function_types() {
    let mut arena = TypeArena::new();
    let a = arena.intern_type_str("A");
    let b = arena.intern_type_str("B");
    let r = arena.intern_type_str("R");
    match arena.get(arena.intern_type_str("R Function(A, B)")) {
        Type::Function { params, return_ } => {
            assert_eq!(params, vec![a, b]);
            assert_eq!(return_, r);
        }
        other => panic!("expected Dart Function type, got {other:?}"),
    }
    assert!(
        matches!(
            arena.get(arena.intern_type_str("Box<R Function(A)>")),
            Type::Apply { .. }
        ),
        "a nested Dart function type must not turn its outer generic into a function"
    );
    let user = arena.intern_type_str("User");
    let int = arena.intern_type_str("int");
    match arena.get(arena.intern_type_str("void Function(User user, int count)")) {
        Type::Function { params, .. } => assert_eq!(params, vec![user, int]),
        other => panic!("expected named Dart Function slots, got {other:?}"),
    }
    match arena.get(arena.intern_type_str("void Function(void Function(User) callback)")) {
        Type::Function { params, .. } => {
            assert!(matches!(arena.get(params[0]), Type::Function { .. }))
        }
        other => panic!("expected nested named Dart Function slot, got {other:?}"),
    }
}

#[test]
fn intern_type_str_parses_representable_go_function_types() {
    let mut arena = TypeArena::new();
    let t = arena.intern_type_str("T");
    let u = arena.intern_type_str("U");
    let r = arena.intern_type_str("R");
    for source in ["func(T) R", "func(value T) R", "func(a, b T, c U) R"] {
        match arena.get(arena.intern_type_str(source)) {
            Type::Function { params, return_ } => {
                let expected = if source.contains("a, b") {
                    vec![t, t, u]
                } else {
                    vec![t]
                };
                assert_eq!(params, expected, "{source}");
                assert_eq!(return_, r, "{source}");
            }
            other => panic!("expected Go Function for {source}, got {other:?}"),
        }
    }
    for unsupported in ["func(...T) R", "func(T) (R, error)"] {
        assert!(
            !matches!(
                arena.get(arena.intern_type_str(unsupported)),
                Type::Function { .. }
            ),
            "{unsupported}: unsupported Go function forms must abstain"
        );
    }
}

#[test]
fn intern_type_str_parses_union() {
    let mut arena = TypeArena::new();
    let a = arena.intern_type_str("A");
    let b = arena.intern_type_str("B");
    // A top-level `|` makes a union, not an opaque class named "A | B".
    match arena.get(arena.intern_type_str("A | B")) {
        Type::Union(arms) => assert_eq!(arms, vec![a, b]),
        other => panic!("expected Union, got {other:?}"),
    }
    // TS pretty-prints unions with a leading pipe; the empty first piece drops.
    match arena.get(arena.intern_type_str("| A | B")) {
        Type::Union(arms) => assert_eq!(arms, vec![a, b]),
        other => panic!("expected Union, got {other:?}"),
    }
    // The arms keep their own structure — a generic arm is an Apply, not a Class.
    match arena.get(arena.intern_type_str(
        "UseQueryReturnType<TData, TError> | UseQueryDefinedReturnType<TData, TError>",
    )) {
        Type::Union(arms) => {
            assert_eq!(arms.len(), 2);
            assert!(matches!(arena.get(arms[0]), Type::Apply { .. }));
            assert!(matches!(arena.get(arms[1]), Type::Apply { .. }));
        }
        other => panic!("expected Union of applies, got {other:?}"),
    }
}

#[test]
fn intern_type_str_parses_tuple() {
    let mut arena = TypeArena::new();
    // A bracket-enclosed, comma-separated `[A, B]` is a tuple, not a class.
    match arena.get(arena.intern_type_str("[Accessor, Setter]")) {
        Type::Tuple(elems) => {
            assert_eq!(elems.len(), 2);
            assert!(matches!(arena.get(elems[0]), Type::Class(q) if q == "Accessor"));
            assert!(matches!(arena.get(elems[1]), Type::Class(q) if q == "Setter"));
        }
        other => panic!("expected Tuple, got {other:?}"),
    }
    // Labeled elements drop their label; nested generics stay structured.
    match arena.get(arena.intern_type_str("[get: Accessor<T>, set: Setter<T>]")) {
        Type::Tuple(elems) => {
            assert_eq!(elems.len(), 2);
            assert!(matches!(arena.get(elems[0]), Type::Apply { .. }));
        }
        other => panic!("expected Tuple of applies, got {other:?}"),
    }
    // A `T[]` array suffix is NOT a tuple — it stays `Array<T>`.
    assert!(matches!(
        arena.get(arena.intern_type_str("User[]")),
        Type::Apply { .. }
    ));
}

#[test]
fn intern_type_str_parses_parenthesized_multi_element_tuples_only() {
    let mut arena = TypeArena::new();
    match arena.get(arena.intern_type_str("(Key, Vec<Inputs>)")) {
        Type::Tuple(elems) => {
            assert_eq!(elems.len(), 2);
            assert!(matches!(arena.get(elems[0]), Type::Class(q) if q == "Key"));
            assert!(matches!(arena.get(elems[1]), Type::Apply { .. }));
        }
        other => panic!("expected parenthesized Tuple, got {other:?}"),
    }
    // Nesting uses the same bounded tuple syntax but stays nested in the type
    // graph; this direct-pattern slice never flattens inner tuple elements.
    match arena.get(arena.intern_type_str("((Key, Inputs), Result)")) {
        Type::Tuple(elems) => {
            assert!(matches!(arena.get(elems[0]), Type::Tuple(_)));
            assert!(matches!(arena.get(elems[1]), Type::Class(q) if q == "Result"));
        }
        other => panic!("expected nested parenthesized Tuple, got {other:?}"),
    }
    match arena.get(arena.intern_type_str("(Wrapper<(Key) -> Result>, Inputs)")) {
        Type::Tuple(elems) => {
            assert_eq!(elems.len(), 2);
            assert!(
                matches!(arena.get(elems[0]), Type::Class(q) if q == "Wrapper<(Key) -> Result>")
            );
            assert!(matches!(arena.get(elems[1]), Type::Class(q) if q == "Inputs"));
        }
        other => panic!("expected parenthesized Tuple, got {other:?}"),
    }
    assert!(matches!(
        arena.get(arena.intern_type_str("(Key, Inputs) -> Result")),
        Type::Function { .. }
    ));
    for opaque in [
        "(Key)",
        "(Key,)",
        "(Key,, Inputs)",
        "(Key, [Inputs}, Result)",
        "(Key, Inputs) Extra",
    ] {
        assert!(
            matches!(arena.get(arena.intern_type_str(opaque)), Type::Class(q) if q == opaque),
            "{opaque} must not be treated as a projectable tuple"
        );
    }
}

#[test]
fn intern_type_str_parses_rust_fixed_array_and_slice() {
    let mut arena = TypeArena::new();
    // `[T; N]` — a fixed-size array. The length is discarded; the element
    // decomposes the same canonical `Array<T>` shape a `T[]` suffix does.
    let array_ty = arena.intern_type_str("[Item; 2]");
    let array_suffix_ty = arena.intern_type_str("Item[]");
    assert_eq!(
        array_ty, array_suffix_ty,
        "[T; N] must intern to the same Array<T> as T[]"
    );
    match arena.get(array_ty) {
        Type::Apply { base, args } => {
            assert!(matches!(arena.get(base), Type::Class(q) if q == "Array"));
            assert_eq!(args.len(), 1);
            assert!(matches!(arena.get(args[0]), Type::Class(q) if q == "Item"));
        }
        other => panic!("expected Apply(Array, [Item]), got {other:?}"),
    }
    // `[T]` — an unsized slice, as it appears behind `&[T]` once the leading
    // reference sigil is peeled. Same canonical Array<T> shape.
    assert_eq!(arena.intern_type_str("[Item]"), array_suffix_ty);
    // `&[T]` — the reference sigil is peeled before the bracket form is seen.
    assert_eq!(arena.intern_type_str("&[Item]"), array_suffix_ty);
    // `&'static [T]` — a lifetime between the sigil and the slice.
    assert_eq!(arena.intern_type_str("&'static [Item]"), array_suffix_ty);
    // A nested generic element stays structured, not flattened to a string.
    match arena.get(arena.intern_type_str("[Vec<Item>; 3]")) {
        Type::Apply { base, args } => {
            assert!(matches!(arena.get(base), Type::Class(q) if q == "Array"));
            assert_eq!(args.len(), 1);
            assert!(matches!(arena.get(args[0]), Type::Apply { .. }));
        }
        other => panic!("expected Apply(Array, [Apply(Vec, [Item])]), got {other:?}"),
    }
}

#[test]
fn intern_type_str_parses_intersection() {
    let mut arena = TypeArena::new();
    let a = arena.intern_type_str("MockInstance");
    let b = arena.intern_type_str("Procedure");
    match arena.get(arena.intern_type_str("MockInstance & Procedure")) {
        Type::Intersection(arms) => assert_eq!(arms, vec![a, b]),
        other => panic!("expected Intersection, got {other:?}"),
    }
}

#[test]
fn intern_type_str_union_intersection_respect_nesting_and_precedence() {
    let mut arena = TypeArena::new();
    // An interior `|` inside generic brackets is NOT a top-level union.
    assert!(matches!(
        arena.get(arena.intern_type_str("Record<string, A | B>")),
        Type::Apply { .. }
    ));
    // `&` binds tighter than `|`: `A & B | C` parses as `(A & B) | C` — a union
    // whose first arm is an intersection.
    match arena.get(arena.intern_type_str("A & B | C")) {
        Type::Union(arms) => {
            assert_eq!(arms.len(), 2);
            assert!(matches!(arena.get(arms[0]), Type::Intersection(_)));
            assert!(matches!(arena.get(arms[1]), Type::Class(q) if q == "C"));
        }
        other => panic!("expected Union, got {other:?}"),
    }
    // A leading `&` at byte 0 is the reference sigil, not an intersection.
    assert!(matches!(
        arena.get(arena.intern_type_str("&User")),
        Type::Class(q) if q == "User"
    ));
    // A single nominal type with no top-level operator is untouched.
    assert!(matches!(
        arena.get(arena.intern_type_str("User")),
        Type::Class(_)
    ));
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
    let Type::Apply {
        base: inner_base,
        args: inner_args,
    } = arena.get(args[0])
    else {
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
    assert_eq!(
        arena.intern_type_str("&Box<C>"),
        arena.intern_type_str("Box<C>")
    );
}

#[test]
fn intern_type_str_strips_pointer_sigil() {
    let arena = TypeArena::new();
    // A leading `*` is a pointer sigil (Go `*fiber.Ctx`, C `*T`), never part of
    // a class name — the receiver interns as the pointee, whose members a
    // pointer walks. A double pointer peels the same way.
    let ctx = arena.class("fiber.Ctx");
    assert_eq!(arena.intern_type_str("*fiber.Ctx"), ctx);
    assert_eq!(arena.intern_type_str("* fiber.Ctx"), ctx);
    assert_eq!(arena.intern_type_str("**fiber.Ctx"), ctx);
    // A pointer to a generic application keeps the Apply.
    assert_eq!(
        arena.intern_type_str("*Box<C>"),
        arena.intern_type_str("Box<C>")
    );
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
            assert_eq!(
                args,
                vec![str_ty],
                "lifetime arg dropped, only `str` remains"
            );
        }
        other => panic!("expected Apply, got {other:?}"),
    }
    // A `'static` lifetime is dropped the same way.
    let cow_static = arena.intern_type_str("Cow<'static, str>");
    assert_eq!(
        cow_static, cow_str,
        "any lifetime arg is dropped identically"
    );
    // An all-lifetime arg list collapses to the bare base class (no type args).
    let only_life = arena.intern_type_str("Ref<'a>");
    assert!(matches!(arena.get(only_life), Type::Class(q) if q == "Ref"));
}

#[test]
fn intern_type_str_distinguishes_interior_ampersand_from_sigil() {
    let arena = TypeArena::new();
    // An interior `&` is a TS intersection operator — `A & B` decomposes into an
    // Intersection of both branches.
    let a = arena.intern_type_str("A");
    let b = arena.intern_type_str("B");
    match arena.get(arena.intern_type_str("A & B")) {
        Type::Intersection(arms) => assert_eq!(arms, vec![a, b]),
        other => panic!("expected Intersection, got {other:?}"),
    }
    // A byte-0 `&` is the reference sigil and is peeled before the operator
    // split, so `&User` interns as the bare referent.
    assert!(matches!(arena.get(arena.intern_type_str("&User")), Type::Class(q) if q == "User"));
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
    assert!(
        matches!(arena.get(arena.intern_type_str("Something")), Type::Class(q) if q == "Something")
    );
    assert!(matches!(arena.get(arena.intern_type_str("anyOf")), Type::Class(q) if q == "anyOf"));
    assert!(
        matches!(arena.get(arena.intern_type_str("SomeType")), Type::Class(q) if q == "SomeType")
    );
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
