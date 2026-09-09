use super::*;

#[test]
fn compiler_labelled_atomic_types_are_captured_without_display_recovery() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../resolution_oracle/atomic_type_fixtures.json"
    ))
    .unwrap();
    for case in cases {
        let source = format!(
            "interface Values {{ value: {}; }}",
            case["syntax"].as_str().unwrap()
        );
        for grammar in [
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
            tree_sitter_typescript::LANGUAGE_TSX,
        ] {
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&grammar.into()).unwrap();
            let tree = parser.parse(&source, None).unwrap();
            assert!(
                !tree.root_node().has_error(),
                "{}: {}",
                source,
                tree.root_node().to_sexp()
            );
            let graph = crate::indexer::lexical::capture(
                tree.root_node(),
                source.as_bytes(),
                "ts",
                &mut vec![],
                &[],
                crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
            )
            .unwrap();
            assert_eq!(graph.types.signatures.len(), 1);
            let actual = match graph.types.signatures[0].result.as_ref().unwrap() {
                TypeExpr::Intrinsic(kind) => serde_json::json!({"intrinsic":kind}),
                TypeExpr::Literal(LitValue::Str(text)) => {
                    serde_json::json!({"string":text.encode_utf16().collect::<Vec<_>>()})
                }
                TypeExpr::Literal(LitValue::Utf16(units)) => serde_json::json!({"string":units}),
                TypeExpr::Literal(LitValue::Number(bits)) => {
                    serde_json::json!({"number":format!("{bits:016x}")})
                }
                TypeExpr::Literal(LitValue::Bool(value)) => serde_json::json!({"boolean":value}),
                TypeExpr::Literal(LitValue::BigInt { negative, words }) => {
                    serde_json::json!({"bigint":{"negative":negative,"words":words}})
                }
                other => panic!("atomic type lost: {source}: {other:?}"),
            };
            assert_eq!(actual, case["expected"], "{source}");
        }
    }
}

#[test]
fn invalid_atomic_syntax_is_not_a_nominal_recovery_hint() {
    let forms = &crate::languages::typescript::flow::ATOMIC_TYPES;
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse("let x = missing;", None).unwrap();
    assert!(matches!(
        capture(tree.root_node(), b"let x = missing;", forms),
        TypeExpr::Unknown
    ));
}
