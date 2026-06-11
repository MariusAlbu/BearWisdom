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
