use super::*;

#[test]
fn profile_borrows_preserve_mutability_and_reject_raw_or_invalid_syntax() {
    let forms = BorrowSyntax {
        node: "reference_expression",
        operand: "value",
        mutable: "mutable_specifier",
        excluded_tokens: &["raw"],
    };
    for (expression, expected) in [
        ("&x", Some(Mutability::Shared)),
        ("&mut x", Some(Mutability::Mutable)),
        ("&raw const x", None),
        ("&raw mut x", None),
        ("&", None),
    ] {
        let source = format!("fn f() {{ target({expression}); }}");
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(&source, None).unwrap();
        let args = tree
            .root_node()
            .named_child(0)
            .unwrap()
            .child_by_field_name("body")
            .unwrap()
            .named_child(0)
            .unwrap()
            .named_child(0)
            .unwrap()
            .child_by_field_name("arguments")
            .unwrap();
        let node = args.named_child(0).unwrap();
        assert_eq!(
            forms.capture(node).map(|(_, mutable)| mutable),
            expected,
            "{expression}: {}",
            node.to_sexp()
        );
    }
}
