use super::*;

#[test]
fn configured_borrow_arguments_keep_nested_operands_and_comments_without_changing_legacy_capture() {
    let source = "fn f() { target(&(x), &mut /* trivia */ y, &&z, &raw const x); }";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let call = tree
        .root_node()
        .named_child(0)
        .unwrap()
        .child_by_field_name("body")
        .unwrap()
        .named_child(0)
        .unwrap()
        .named_child(0)
        .unwrap();
    let args = extract_call_args_with_borrows(
        &call,
        source.as_bytes(),
        crate::languages::rust_lang::namespaces::FORMS.borrows,
    );
    assert_eq!(args.len(), 4);
    let mut leaves = Vec::new();
    for arg in &args {
        arg.visit_identifiers(&mut |span| {
            leaves.push(&source[span.start as usize..span.end as usize])
        });
    }
    assert_eq!(leaves, ["x", "y", "z"]);
    let CallArg::BorrowAt { span, expr } = &args[2] else {
        panic!("nested borrow root");
    };
    assert_eq!(&source[span.start as usize..span.end as usize], "&&z");
    let CallArg::BorrowAt { span: inner, expr } = expr.as_ref() else {
        panic!("second borrow layer");
    };
    assert_eq!(&source[inner.start as usize..inner.end as usize], "&z");
    assert!(matches!(expr.as_ref(), CallArg::IdentAt(_)));
    assert_eq!(args[3], CallArg::Other);
    assert_eq!(
        extract_call_args(&call, source.as_bytes()),
        vec![CallArg::Other; 4]
    );
}

#[test]
fn recursive_identifier_spans_survive_parentheses_await_and_comments_in_ts_and_js() {
    let source = "f(/* ignored */ (alpha), await beta, [/* ignored */ gamma], yes ? delta : epsilon, ...rest, xs[index], left + right);";
    for language in [
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        tree_sitter_javascript::LANGUAGE.into(),
    ] {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(source, None).unwrap();
        let call = tree
            .root_node()
            .named_child(0)
            .unwrap()
            .named_child(0)
            .unwrap();
        let args = extract_call_args(&call, source.as_bytes());
        assert_eq!(args.len(), 7, "comments are not argument positions");
        let mut identifiers = Vec::new();
        for arg in &args {
            arg.visit_identifiers(&mut |span| {
                identifiers.push(&source[span.start as usize..span.end as usize])
            });
        }
        assert_eq!(
            identifiers,
            [
                "alpha", "beta", "gamma", "delta", "epsilon", "rest", "xs", "index", "left",
                "right"
            ]
        );
        assert!(matches!(args[0], CallArg::IdentAt(_)));
        assert!(matches!(args[1], CallArg::Await { .. }));
    }
}
