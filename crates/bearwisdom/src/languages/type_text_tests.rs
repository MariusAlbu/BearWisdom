use crate::type_checker::core::types::{Type, TypeArena};

#[test]
fn source_type_text_parses_function_type() {
    let mut arena = TypeArena::new();
    let f = crate::languages::type_text::intern_test_type_text(&arena, "() => User");
    match arena.get(f) {
        Type::Function { params, return_ } => {
            assert!(params.is_empty());
            assert!(matches!(arena.get(return_), Type::Class(q) if q == "User"));
        }
        other => panic!("expected Function, got {other:?}"),
    }
    // Params with annotations and a generic return are still a function type.
    assert!(matches!(
        arena.get(crate::languages::type_text::intern_test_type_text(
            &arena,
            "(x: number) => Box<User>"
        )),
        Type::Function { .. }
    ));
    // A generic that merely carries a function-typed arg must NOT be read as a
    // function type — the arrow is nested, not top-level.
    assert!(!matches!(
        arena.get(crate::languages::type_text::intern_test_type_text(
            &arena,
            "Foo<() => void>"
        )),
        Type::Function { .. }
    ));
    // A plain nominal type is unaffected.
    assert!(matches!(
        arena.get(crate::languages::type_text::intern_test_type_text(
            &arena, "User"
        )),
        Type::Class(_)
    ));
}

#[test]
fn source_type_text_parses_arrow_return_function_type() {
    let mut arena = TypeArena::new();
    // Rust `Fn() -> T`, Kotlin / Swift `(T) -> R`, and `impl Fn(..) -> T` all
    // carry a top-level `->` whose right side is the return type.
    for s in ["Fn() -> User", "(x: I32) -> User", "impl Fn() -> User"] {
        match arena.get(crate::languages::type_text::intern_test_type_text(
            &arena, s,
        )) {
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
        arena.get(crate::languages::type_text::intern_test_type_text(
            &arena,
            "Box<dyn Fn() -> User>"
        )),
        Type::Function { .. }
    ));
}

#[test]
fn source_type_text_preserves_function_param_types() {
    let mut arena = TypeArena::new();
    // TS `(name: T) =>` — the param annotation is peeled to its bare type, so
    // the param interns as `Class("T")` (rebind lifts Class→Generic later).
    let t = crate::languages::type_text::intern_test_type_text(&arena, "T");
    let f = crate::languages::type_text::intern_test_type_text(&arena, "(value: T) => U");
    match arena.get(f) {
        Type::Function { params, return_ } => {
            assert_eq!(params, vec![t]);
            assert!(matches!(arena.get(return_), Type::Class(q) if q == "U"));
        }
        other => panic!("expected Function, got {other:?}"),
    }
    // Rust `Fn(T) -> U` — the param is bare (no colon), so the whole piece is
    // the type. The param list is the first top-level parens after `Fn`.
    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena,
        "Fn(T) -> U",
    )) {
        Type::Function { params, return_ } => {
            assert_eq!(
                params,
                vec![crate::languages::type_text::intern_test_type_text(
                    &arena, "T"
                )]
            );
            assert!(matches!(arena.get(return_), Type::Class(q) if q == "U"));
        }
        other => panic!("expected Function, got {other:?}"),
    }
}

#[test]
fn source_type_text_parses_fixed_arity_python_callable_annotations() {
    let mut arena = TypeArena::new();
    let a = crate::languages::type_text::intern_test_type_text(&arena, "A");
    let b = crate::languages::type_text::intern_test_type_text(&arena, "B");
    let r = crate::languages::type_text::intern_test_type_text(&arena, "R");
    for source in [
        "Callable[[A, B], R]",
        "typing.Callable[[A, B], R]",
        "collections.abc.Callable[[A, B], R]",
    ] {
        match arena.get(crate::languages::type_text::intern_test_type_text(
            &arena, source,
        )) {
            Type::Function { params, return_ } => {
                assert_eq!(params, vec![a, b], "{source}");
                assert_eq!(return_, r, "{source}");
            }
            other => panic!("expected Python Callable Function for {source}, got {other:?}"),
        }
    }

    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena,
        "typing.Callable[[Callable[[A], B], list[Item]], Result]",
    )) {
        Type::Function { params, return_ } => {
            assert_eq!(params.len(), 2);
            assert!(matches!(arena.get(params[0]), Type::Function { .. }));
            assert!(matches!(arena.get(params[1]), Type::Apply { .. }));
            assert!(matches!(arena.get(return_), Type::Class(name) if name == "Result"));
        }
        other => panic!("expected nested Python Callable Function, got {other:?}"),
    }

    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena,
        "collections.abc.Callable[[], None]",
    )) {
        Type::Function { params, return_ } => {
            assert!(params.is_empty());
            assert!(matches!(arena.get(return_), Type::Class(name) if name == "None"));
        }
        other => panic!("expected zero-argument Python Callable Function, got {other:?}"),
    }
}

#[test]
fn source_type_text_keeps_unmodeled_python_callable_forms_opaque() {
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
                arena.get(crate::languages::type_text::intern_test_type_text(
                    &arena, source
                )),
                Type::Function { .. }
            ),
            "{source} must stay opaque without callable binder identity"
        );
    }
}

#[test]
fn source_type_text_parses_dart_return_first_function_types() {
    let mut arena = TypeArena::new();
    let a = crate::languages::type_text::intern_test_type_text(&arena, "A");
    let b = crate::languages::type_text::intern_test_type_text(&arena, "B");
    let r = crate::languages::type_text::intern_test_type_text(&arena, "R");
    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena,
        "R Function(A, B)",
    )) {
        Type::Function { params, return_ } => {
            assert_eq!(params, vec![a, b]);
            assert_eq!(return_, r);
        }
        other => panic!("expected Dart Function type, got {other:?}"),
    }
    assert!(
        matches!(
            arena.get(crate::languages::type_text::intern_test_type_text(
                &arena,
                "Box<R Function(A)>"
            )),
            Type::Apply { .. }
        ),
        "a nested Dart function type must not turn its outer generic into a function"
    );
    let user = crate::languages::type_text::intern_test_type_text(&arena, "User");
    let int = crate::languages::type_text::intern_test_type_text(&arena, "int");
    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena,
        "void Function(User user, int count)",
    )) {
        Type::Function { params, .. } => assert_eq!(params, vec![user, int]),
        other => panic!("expected named Dart Function slots, got {other:?}"),
    }
    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena,
        "void Function(void Function(User) callback)",
    )) {
        Type::Function { params, .. } => {
            assert!(matches!(arena.get(params[0]), Type::Function { .. }))
        }
        other => panic!("expected nested named Dart Function slot, got {other:?}"),
    }
}

#[test]
fn source_type_text_parses_representable_go_function_types() {
    let mut arena = TypeArena::new();
    let t = crate::languages::type_text::intern_test_type_text(&arena, "T");
    let u = crate::languages::type_text::intern_test_type_text(&arena, "U");
    let r = crate::languages::type_text::intern_test_type_text(&arena, "R");
    for source in ["func(T) R", "func(value T) R", "func(a, b T, c U) R"] {
        match arena.get(crate::languages::type_text::intern_test_type_text(
            &arena, source,
        )) {
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
                arena.get(crate::languages::type_text::intern_test_type_text(
                    &arena,
                    unsupported
                )),
                Type::Function { .. }
            ),
            "{unsupported}: unsupported Go function forms must abstain"
        );
    }
}

#[test]
fn source_type_text_parses_union() {
    let mut arena = TypeArena::new();
    let a = crate::languages::type_text::intern_test_type_text(&arena, "A");
    let b = crate::languages::type_text::intern_test_type_text(&arena, "B");
    // A top-level `|` makes a union, not an opaque class named "A | B".
    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena, "A | B",
    )) {
        Type::Union(arms) => assert_eq!(arms, vec![a, b]),
        other => panic!("expected Union, got {other:?}"),
    }
    // TS pretty-prints unions with a leading pipe; the empty first piece drops.
    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena, "| A | B",
    )) {
        Type::Union(arms) => assert_eq!(arms, vec![a, b]),
        other => panic!("expected Union, got {other:?}"),
    }
    // The arms keep their own structure — a generic arm is an Apply, not a Class.
    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena,
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
fn source_type_text_parses_tuple() {
    let mut arena = TypeArena::new();
    // A bracket-enclosed, comma-separated `[A, B]` is a tuple, not a class.
    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena,
        "[Accessor, Setter]",
    )) {
        Type::Tuple(elems) => {
            assert_eq!(elems.len(), 2);
            assert!(matches!(arena.get(elems[0]), Type::Class(q) if q == "Accessor"));
            assert!(matches!(arena.get(elems[1]), Type::Class(q) if q == "Setter"));
        }
        other => panic!("expected Tuple, got {other:?}"),
    }
    // Labeled elements drop their label; nested generics stay structured.
    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena,
        "[get: Accessor<T>, set: Setter<T>]",
    )) {
        Type::Tuple(elems) => {
            assert_eq!(elems.len(), 2);
            assert!(matches!(arena.get(elems[0]), Type::Apply { .. }));
        }
        other => panic!("expected Tuple of applies, got {other:?}"),
    }
    // A `T[]` array suffix is NOT a tuple — it stays `Array<T>`.
    assert!(matches!(
        arena.get(crate::languages::type_text::intern_test_type_text(
            &arena, "User[]"
        )),
        Type::Apply { .. }
    ));
}

#[test]
fn source_type_text_parses_parenthesized_multi_element_tuples_only() {
    let mut arena = TypeArena::new();
    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena,
        "(Key, Vec<Inputs>)",
    )) {
        Type::Tuple(elems) => {
            assert_eq!(elems.len(), 2);
            assert!(matches!(arena.get(elems[0]), Type::Class(q) if q == "Key"));
            assert!(matches!(arena.get(elems[1]), Type::Apply { .. }));
        }
        other => panic!("expected parenthesized Tuple, got {other:?}"),
    }
    // Nesting uses the same bounded tuple syntax but stays nested in the type
    // graph; this direct-pattern slice never flattens inner tuple elements.
    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena,
        "((Key, Inputs), Result)",
    )) {
        Type::Tuple(elems) => {
            assert!(matches!(arena.get(elems[0]), Type::Tuple(_)));
            assert!(matches!(arena.get(elems[1]), Type::Class(q) if q == "Result"));
        }
        other => panic!("expected nested parenthesized Tuple, got {other:?}"),
    }
    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena,
        "(Wrapper<(Key) -> Result>, Inputs)",
    )) {
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
        arena.get(crate::languages::type_text::intern_test_type_text(
            &arena,
            "(Key, Inputs) -> Result"
        )),
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
            matches!(arena.get(crate::languages::type_text::intern_test_type_text(&arena, opaque)), Type::Class(q) if q == opaque),
            "{opaque} must not be treated as a projectable tuple"
        );
    }
}

#[test]
fn source_type_text_parses_rust_fixed_array_and_slice() {
    let mut arena = TypeArena::new();
    // `[T; N]` — a fixed-size array. The length is discarded; the element
    // decomposes the same canonical `Array<T>` shape a `T[]` suffix does.
    let array_ty = crate::languages::type_text::intern_test_type_text(&arena, "[Item; 2]");
    let array_suffix_ty = crate::languages::type_text::intern_test_type_text(&arena, "Item[]");
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
    assert_eq!(
        crate::languages::type_text::intern_test_type_text(&arena, "[Item]"),
        array_suffix_ty
    );
    // `&[T]` — the reference sigil is peeled before the bracket form is seen.
    assert_eq!(
        crate::languages::type_text::intern_test_type_text(&arena, "&[Item]"),
        array_suffix_ty
    );
    // `&'static [T]` — a lifetime between the sigil and the slice.
    assert_eq!(
        crate::languages::type_text::intern_test_type_text(&arena, "&'static [Item]"),
        array_suffix_ty
    );
    // A nested generic element stays structured, not flattened to a string.
    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena,
        "[Vec<Item>; 3]",
    )) {
        Type::Apply { base, args } => {
            assert!(matches!(arena.get(base), Type::Class(q) if q == "Array"));
            assert_eq!(args.len(), 1);
            assert!(matches!(arena.get(args[0]), Type::Apply { .. }));
        }
        other => panic!("expected Apply(Array, [Apply(Vec, [Item])]), got {other:?}"),
    }
}

#[test]
fn source_type_text_parses_intersection() {
    let mut arena = TypeArena::new();
    let a = crate::languages::type_text::intern_test_type_text(&arena, "MockInstance");
    let b = crate::languages::type_text::intern_test_type_text(&arena, "Procedure");
    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena,
        "MockInstance & Procedure",
    )) {
        Type::Intersection(arms) => assert_eq!(arms, vec![a, b]),
        other => panic!("expected Intersection, got {other:?}"),
    }
}

#[test]
fn source_type_text_union_intersection_respect_nesting_and_precedence() {
    let mut arena = TypeArena::new();
    // An interior `|` inside generic brackets is NOT a top-level union.
    assert!(matches!(
        arena.get(crate::languages::type_text::intern_test_type_text(
            &arena,
            "Record<string, A | B>"
        )),
        Type::Apply { .. }
    ));
    // `&` binds tighter than `|`: `A & B | C` parses as `(A & B) | C` — a union
    // whose first arm is an intersection.
    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena,
        "A & B | C",
    )) {
        Type::Union(arms) => {
            assert_eq!(arms.len(), 2);
            assert!(matches!(arena.get(arms[0]), Type::Intersection(_)));
            assert!(matches!(arena.get(arms[1]), Type::Class(q) if q == "C"));
        }
        other => panic!("expected Union, got {other:?}"),
    }
    // A leading `&` at byte 0 is the reference sigil, not an intersection.
    assert!(matches!(
        arena.get(crate::languages::type_text::intern_test_type_text(&arena, "&User")),
        Type::Class(q) if q == "User"
    ));
    // A single nominal type with no top-level operator is untouched.
    assert!(matches!(
        arena.get(crate::languages::type_text::intern_test_type_text(
            &arena, "User"
        )),
        Type::Class(_)
    ));
}

#[test]
fn source_type_text_handles_simple_class() {
    let arena = TypeArena::new();
    let id = crate::languages::type_text::intern_test_type_text(&arena, "User");
    match arena.get(id) {
        Type::Class(q) => assert_eq!(q, "User"),
        other => panic!("expected Class, got {other:?}"),
    }
}

#[test]
fn source_type_text_trims_whitespace() {
    let arena = TypeArena::new();
    let trimmed = crate::languages::type_text::intern_test_type_text(&arena, "  Foo  ");
    let direct = arena.class("Foo");
    assert_eq!(trimmed, direct);
}

#[test]
fn source_type_text_decomposes_generic_application() {
    let arena = TypeArena::new();
    let id = crate::languages::type_text::intern_test_type_text(&arena, "Repository<User>");
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
fn source_type_text_decomposes_multi_arg_generic() {
    let arena = TypeArena::new();
    let id = crate::languages::type_text::intern_test_type_text(&arena, "Map<K, V>");
    let Type::Apply { base, args } = arena.get(id) else {
        panic!("expected Apply");
    };
    assert_eq!(arena.get(base), Type::Class("Map".to_string()));
    assert_eq!(args.len(), 2);
    assert_eq!(arena.get(args[0]), Type::Class("K".to_string()));
    assert_eq!(arena.get(args[1]), Type::Class("V".to_string()));
}

#[test]
fn source_type_text_handles_nested_generics() {
    let arena = TypeArena::new();
    let id = crate::languages::type_text::intern_test_type_text(&arena, "Promise<Result<Ok, Err>>");
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
fn source_type_text_dedups_identical_apply() {
    let arena = TypeArena::new();
    let a = crate::languages::type_text::intern_test_type_text(&arena, "Repository<User>");
    let b = crate::languages::type_text::intern_test_type_text(&arena, "Repository<User>");
    assert_eq!(a, b);
}

#[test]
fn source_type_text_accepts_scala_bracket_style() {
    let arena = TypeArena::new();
    let id = crate::languages::type_text::intern_test_type_text(&arena, "Map[K, V]");
    let Type::Apply { base, args } = arena.get(id) else {
        panic!("expected Apply for Scala-style brackets");
    };
    assert_eq!(arena.get(base), Type::Class("Map".to_string()));
    assert_eq!(args.len(), 2);
}

#[test]
fn source_type_text_falls_back_to_class_for_unbalanced() {
    let arena = TypeArena::new();
    let id = crate::languages::type_text::intern_test_type_text(&arena, "Foo<Bar");
    // Unbalanced — entire string becomes a Class.
    assert_eq!(arena.get(id), Type::Class("Foo<Bar".to_string()));
}

#[test]
fn source_type_text_falls_back_for_anonymous_generic() {
    let arena = TypeArena::new();
    let id = crate::languages::type_text::intern_test_type_text(&arena, "<Bar>");
    // Empty head — fallback to Class on the raw input.
    assert_eq!(arena.get(id), Type::Class("<Bar>".to_string()));
}

#[test]
fn source_type_text_falls_back_on_post_bracket_text() {
    let arena = TypeArena::new();
    let id = crate::languages::type_text::intern_test_type_text(&arena, "Foo<Bar>.Baz");
    // Anything after the closing bracket isn't first-class — fallback.
    assert_eq!(arena.get(id), Type::Class("Foo<Bar>.Baz".to_string()));
}

#[test]
fn source_type_text_empty_string_falls_back_to_class() {
    let arena = TypeArena::new();
    let id = crate::languages::type_text::intern_test_type_text(&arena, "");
    assert_eq!(arena.get(id), Type::Class("".to_string()));
}

#[test]
fn format_type_round_trips_through_arena() {
    let arena = TypeArena::new();
    let original = "Outer<Middle<Inner>>";
    let id = crate::languages::type_text::intern_test_type_text(&arena, original);
    let formatted = arena.format_type(id);
    let id2 = crate::languages::type_text::intern_test_type_text(&arena, &formatted);
    assert_eq!(id, id2);
}

#[test]
fn source_type_text_strips_reference_sigil() {
    let arena = TypeArena::new();
    // A leading `&` (with an optional lifetime and `mut`) is a reference sigil,
    // never part of a class name — strip it so the receiver interns as the
    // referent, matching the type a `&C` / `&mut C` binding members against.
    let c = arena.class("C");
    assert_eq!(
        crate::languages::type_text::intern_test_type_text(&arena, "&C"),
        c
    );
    assert_eq!(
        crate::languages::type_text::intern_test_type_text(&arena, "&mut C"),
        c
    );
    assert_eq!(
        crate::languages::type_text::intern_test_type_text(&arena, "&'a C"),
        c
    );
    assert_eq!(
        crate::languages::type_text::intern_test_type_text(&arena, "& 'a mut C"),
        c
    );
    // A reference to a generic application strips the sigil and keeps the Apply.
    assert_eq!(
        crate::languages::type_text::intern_test_type_text(&arena, "&Box<C>"),
        crate::languages::type_text::intern_test_type_text(&arena, "Box<C>")
    );
}

#[test]
fn source_type_text_strips_pointer_sigil() {
    let arena = TypeArena::new();
    // A leading `*` is a pointer sigil (Go `*fiber.Ctx`, C `*T`), never part of
    // a class name — the receiver interns as the pointee, whose members a
    // pointer walks. A double pointer peels the same way.
    let ctx = arena.class("fiber.Ctx");
    assert_eq!(
        crate::languages::type_text::intern_test_type_text(&arena, "*fiber.Ctx"),
        ctx
    );
    assert_eq!(
        crate::languages::type_text::intern_test_type_text(&arena, "* fiber.Ctx"),
        ctx
    );
    assert_eq!(
        crate::languages::type_text::intern_test_type_text(&arena, "**fiber.Ctx"),
        ctx
    );
    // A pointer to a generic application keeps the Apply.
    assert_eq!(
        crate::languages::type_text::intern_test_type_text(&arena, "*Box<C>"),
        crate::languages::type_text::intern_test_type_text(&arena, "Box<C>")
    );
}

#[test]
fn source_type_text_drops_leading_lifetime_arg() {
    let arena = TypeArena::new();
    // A generic argument that is a lifetime (`'a`, `'static`) is never a type —
    // it is dropped from the Apply's args so a wrapper whose first param is a
    // lifetime (`Cow<'a, str>`) interns as a SINGLE-type-arg application, the
    // shape the single-inner-wrapper peel projects through.
    let str_ty = arena.class("str");
    let cow_str = crate::languages::type_text::intern_test_type_text(&arena, "Cow<'a, str>");
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
    let cow_static =
        crate::languages::type_text::intern_test_type_text(&arena, "Cow<'static, str>");
    assert_eq!(
        cow_static, cow_str,
        "any lifetime arg is dropped identically"
    );
    // An all-lifetime arg list collapses to the bare base class (no type args).
    let only_life = crate::languages::type_text::intern_test_type_text(&arena, "Ref<'a>");
    assert!(matches!(arena.get(only_life), Type::Class(q) if q == "Ref"));
}

#[test]
fn source_type_text_distinguishes_interior_ampersand_from_sigil() {
    let arena = TypeArena::new();
    // An interior `&` is a TS intersection operator — `A & B` decomposes into an
    // Intersection of both branches.
    let a = crate::languages::type_text::intern_test_type_text(&arena, "A");
    let b = crate::languages::type_text::intern_test_type_text(&arena, "B");
    match arena.get(crate::languages::type_text::intern_test_type_text(
        &arena, "A & B",
    )) {
        Type::Intersection(arms) => assert_eq!(arms, vec![a, b]),
        other => panic!("expected Intersection, got {other:?}"),
    }
    // A byte-0 `&` is the reference sigil and is peeled before the operator
    // split, so `&User` interns as the bare referent.
    assert!(
        matches!(arena.get(crate::languages::type_text::intern_test_type_text(&arena, "&User")), Type::Class(q) if q == "User")
    );
}

#[test]
fn source_type_text_strips_opaque_existential_prefix() {
    let arena = TypeArena::new();
    // A leading `some `/`any ` is Swift's opaque/existential keyword prefix,
    // never part of a class name — strip it so a value typed `some Greet` /
    // `any Greet` interns as the bare protocol `Greet`, the base under which a
    // protocol extension files its default members.
    let greet = arena.class("Greet");
    assert_eq!(
        crate::languages::type_text::intern_test_type_text(&arena, "some Greet"),
        greet
    );
    assert_eq!(
        crate::languages::type_text::intern_test_type_text(&arena, "any Greet"),
        greet
    );
    // Extra interior whitespace is tolerated (the prefix is keyword + space).
    assert_eq!(
        crate::languages::type_text::intern_test_type_text(&arena, "some   Greet"),
        greet
    );
    // A constrained existential keeps its application: `any Collection<Int>`
    // strips the keyword and re-interns the constrained type.
    assert_eq!(
        crate::languages::type_text::intern_test_type_text(&arena, "any Collection<Int>"),
        crate::languages::type_text::intern_test_type_text(&arena, "Collection<Int>")
    );
}

#[test]
fn source_type_text_does_not_strip_some_any_as_type_name_prefix() {
    let arena = TypeArena::new();
    // `some`/`any` are only stripped as standalone keyword prefixes (followed by
    // whitespace then a type). A class whose NAME begins with those letters but
    // is not the bare keyword (`Something`, `anyOf`, `SomeType`) is untouched —
    // no word boundary, no strip.
    assert!(
        matches!(arena.get(crate::languages::type_text::intern_test_type_text(&arena, "Something")), Type::Class(q) if q == "Something")
    );
    assert!(
        matches!(arena.get(crate::languages::type_text::intern_test_type_text(&arena, "anyOf")), Type::Class(q) if q == "anyOf")
    );
    assert!(
        matches!(arena.get(crate::languages::type_text::intern_test_type_text(&arena, "SomeType")), Type::Class(q) if q == "SomeType")
    );
}
