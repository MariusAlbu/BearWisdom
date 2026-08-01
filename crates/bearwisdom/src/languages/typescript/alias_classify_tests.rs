use super::*;
use crate::types::AliasTarget;

/// Parse `src` as TypeScript, locate the first `type_alias_declaration`, and
/// classify the type expression on the right of `=`. Returns the captured
/// [`AliasTarget`] for the alias's RHS.
fn classify(src: &str) -> AliasTarget {
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).unwrap();
    let tree = parser.parse(src, None).unwrap();
    let root = tree.root_node();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "type_alias_declaration" {
            let value = child
                .child_by_field_name("value")
                .expect("type_alias_declaration has a value field");
            return classify_alias_target(&value, src.as_bytes());
        }
    }
    panic!("no type_alias_declaration in source");
}

#[test]
fn tuple_captures_element_heads_by_position() {
    // Unlabeled tuple — each named element's head type, in order.
    assert_eq!(
        classify("type Pair = [Foo, Bar];"),
        AliasTarget::Tuple(vec!["Foo".to_string(), "Bar".to_string()])
    );
    // Labeled tuple (`[get: Accessor<T>, set: Setter<T>]`) — labels dropped, the
    // generic element heads kept. This is the solid-js `Signal<T>` shape.
    assert_eq!(
        classify("type Signal<T> = [get: Accessor<T>, set: Setter<T>];"),
        AliasTarget::Tuple(vec!["Accessor".to_string(), "Setter".to_string()])
    );
}

#[test]
fn conditional_captures_infer_binding_in_generic_extends() {
    // `type Elem<T> = T extends Array<infer U> ? U : never` — the `infer U`
    // in the extends clause's generic argument is captured as ("U", 0): the
    // variable name plus its 0-based slot in `Array<...>`'s type arguments.
    let target = classify("type Elem<T> = T extends Array<infer U> ? U : never;");
    match target {
        AliasTarget::Conditional {
            check,
            extends,
            true_branch,
            false_branch,
            infer_binding,
        } => {
            assert_eq!(check, "T");
            assert_eq!(extends, "Array");
            assert_eq!(true_branch, "U");
            assert_eq!(false_branch, "never");
            assert_eq!(infer_binding, Some(("U".to_string(), 0)));
        }
        other => panic!("expected Conditional, got {other:?}"),
    }
}

#[test]
fn conditional_without_infer_has_none_binding() {
    // A plain conditional with no `infer` records `infer_binding: None`.
    let target = classify("type Cond = Foo extends string ? A : B;");
    match target {
        AliasTarget::Conditional { infer_binding, .. } => {
            assert_eq!(infer_binding, None);
        }
        other => panic!("expected Conditional, got {other:?}"),
    }
}

#[test]
fn conditional_with_multiple_infer_declines_binding() {
    // `T extends Map<infer K, infer V> ? K : never` carries two `infer`
    // captures in one clause; the single-capture recorder declines (the
    // expander does not unify multi-capture patterns), so `infer_binding`
    // is None while the four head names are still captured.
    let target = classify("type Keys<T> = T extends Map<infer K, infer V> ? K : never;");
    match target {
        AliasTarget::Conditional {
            extends,
            infer_binding,
            ..
        } => {
            assert_eq!(extends, "Map");
            assert_eq!(infer_binding, None);
        }
        other => panic!("expected Conditional, got {other:?}"),
    }
}

#[test]
fn conditional_infer_in_second_slot_records_slot_index() {
    // `T extends Map<string, infer V> ? V : never` — the lone `infer` is the
    // second type argument, so the captured slot is 1.
    let target = classify("type Val<T> = T extends Map<string, infer V> ? V : never;");
    match target {
        AliasTarget::Conditional { infer_binding, .. } => {
            assert_eq!(infer_binding, Some(("V".to_string(), 1)));
        }
        other => panic!("expected Conditional, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Intersection with anonymous mapped branch
// ---------------------------------------------------------------------------

#[test]
fn standalone_mapped_type_classifies_as_mapped() {
    // `type T<Q> = { [P in keyof Q]: string }` — a standalone mapped type.
    // Should classify as `Mapped { source: "Q", ... }`.
    let src = "type T<Q> = { [P in keyof Q]: string };";
    let target = classify(src);
    match target {
        AliasTarget::Mapped { source, .. } => {
            assert_eq!(source, "Q");
        }
        other => panic!("expected Mapped(Q) for standalone mapped type, got {other:?}"),
    }
}

#[test]
fn intersection_with_only_anonymous_mapped_branch_classifies_as_mapped() {
    // `type T<Q> = { own: string } & { [P in keyof Q]: V }` — both branches
    // are anonymous (no head name). The mapped branch's source `Q` is surfaced
    // as a `Mapped` alias so the chain walker's `mapped_source_type` can follow
    // through to `Q`'s concrete type rather than silently dropping the branch.
    let src = "type T<Q> = { own: string } & { [P in keyof Q]: string };";
    let target = classify(src);
    match target {
        AliasTarget::Mapped { source, .. } => {
            assert_eq!(source, "Q");
        }
        other => panic!("expected Mapped(Q), got {other:?}"),
    }
}

#[test]
fn intersection_with_named_branch_and_mapped_carries_both() {
    // `type T = Named & { [K in keyof Q]: V }` has a NAMED branch (`Named`) AND a
    // mapped branch — classify as `IntersectionMapped` so member lookup tries the
    // named branch (`lookup_member_on_intersection`) AND the mapped source
    // (`mapped_source_type`), dropping neither half.
    let src = "type T = Named & { [K in keyof Q]: string };";
    let target = classify(src);
    match target {
        AliasTarget::IntersectionMapped {
            branches, source, ..
        } => {
            assert!(
                branches.contains(&"Named".to_string()),
                "expected Named branch, got {branches:?}"
            );
            assert_eq!(source, "Q", "expected mapped source Q, got {source:?}");
        }
        other => panic!("expected IntersectionMapped, got {other:?}"),
    }
}

#[test]
fn nested_intersection_captures_deep_named_branch() {
    // The vitest `Mock` shape: `A & (cond) & { [P in keyof T]: T[P] }` parses
    // left-associatively as `(A & cond) & {mapped}`. The flatten must reach the
    // nested named branch `A` (e.g. MockInstance) and still surface the mapped
    // source — IntersectionMapped carrying both.
    let src = "type T = A & (X extends Y ? P : Q) & { [P in keyof K]: K[P] };";
    let target = classify(src);
    match target {
        AliasTarget::IntersectionMapped { branches, .. } => {
            assert!(
                branches.contains(&"A".to_string()),
                "nested named branch A must survive the flatten, got {branches:?}"
            );
        }
        other => panic!("expected IntersectionMapped, got {other:?}"),
    }
}

#[test]
fn intersection_with_all_anonymous_object_branches_and_no_mapped_stays_object() {
    // `type T = { a: string } & { b: number }` — two anonymous object branches,
    // no mapped clause. Stays `Object` since both branches' members are already
    // flattened onto `T` by `recurse_for_object_types`.
    let src = "type T = { a: string } & { b: number };";
    let target = classify(src);
    // Should NOT be Mapped (no mapped clause) — either Object or Intersection([]).
    assert!(
        !matches!(target, AliasTarget::Mapped { .. }),
        "plain anonymous intersection should not be Mapped; got {target:?}"
    );
}

// ---------------------------------------------------------------------------
// Callable alias — function_type return-head capture
// ---------------------------------------------------------------------------

#[test]
fn callable_alias_captures_return_head_as_application() {
    // `type Accessor<T> = () => T` — the call result is the generic param `T`.
    // Capturing it as `Application { root: "T", args: [] }` lets alias-expansion
    // substitute the application's arg, so `Accessor<QueryClient>` → `QueryClient`.
    let target = classify("type Accessor<T> = () => T;");
    match target {
        AliasTarget::Application { root, args } => {
            assert_eq!(root, "T");
            assert!(args.is_empty());
        }
        other => panic!("expected Application {{ root: \"T\" }}, got {other:?}"),
    }
}

#[test]
fn callable_alias_with_nominal_return_captures_that_head() {
    // `type Lazy = () => User` — nominal return type captured as the root.
    let target = classify("type Lazy = () => User;");
    match target {
        AliasTarget::Application { root, args } => {
            assert_eq!(root, "User");
            assert!(args.is_empty());
        }
        other => panic!("expected Application {{ root: \"User\" }}, got {other:?}"),
    }
}

#[test]
fn callable_alias_with_params_still_captures_return() {
    // `type Mapper<T> = (x: number) => T` — params don't change the return head.
    let target = classify("type Mapper<T> = (x: number) => T;");
    match target {
        AliasTarget::Application { root, .. } => assert_eq!(root, "T"),
        other => panic!("expected Application {{ root: \"T\" }}, got {other:?}"),
    }
}

#[test]
fn returntype_of_typeof_captures_the_value_as_arg() {
    // `type Logger = ReturnType<typeof createScopedLogger>` — the `typeof` arg is
    // captured as the single argument so the ReturnType intrinsic resolves it to
    // createScopedLogger's return type (rather than a dead `ReturnType` class).
    let target = classify("type Logger = ReturnType<typeof createScopedLogger>;");
    match target {
        AliasTarget::Application { root, args } => {
            assert_eq!(root, "ReturnType");
            assert_eq!(args, vec!["createScopedLogger".to_string()]);
        }
        other => panic!(
            "expected Application {{ root: \"ReturnType\", args: [createScopedLogger] }}, got {other:?}"
        ),
    }
}

#[test]
fn non_single_head_return_stays_opaque() {
    // `type F = () => A | B` — union return has no single root; must stay Other.
    let target = classify("type F = () => A | B;");
    assert!(
        matches!(target, AliasTarget::Other),
        "union return should stay Other, got {target:?}"
    );
}

#[test]
fn a_mapped_type_over_a_named_union_records_that_union_as_its_source() {
    // `{ [K in Keys]: V }` names its source directly rather than through
    // `keyof`. Dropping it leaves the mapped type sourceless, so none of the
    // members it generates can resolve.
    let r = crate::languages::typescript::extract::extract(
        "export type Keys = 'click' | 'change'
         export type FireObject = { [K in Keys]: (el: string) => boolean }
",
        false,
    );
    let target = r
        .alias_targets
        .iter()
        .find(|(n, _)| n == "FireObject")
        .map(|(_, t)| t.clone())
        .expect("the mapped alias is captured");
    match target {
        crate::types::AliasTarget::Mapped { source, .. } => assert_eq!(source, "Keys"),
        other => panic!("expected a Mapped target, got {other:?}"),
    }
    // Its source union keeps the literal branches that form the key set.
    let keys = r
        .alias_targets
        .iter()
        .find(|(n, _)| n == "Keys")
        .map(|(_, t)| t.clone())
        .expect("the union alias is captured");
    match keys {
        crate::types::AliasTarget::Union(branches) => {
            assert_eq!(branches, vec!["'click'".to_string(), "'change'".to_string()])
        }
        other => panic!("expected a Union target, got {other:?}"),
    }
}
