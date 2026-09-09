use super::*;

#[test]
fn unsupported_object_members_are_not_silently_empty_objects() {
    for source in [
        "type X = { method<T>(v: T): T }",
        "type X = { new(): X }",
        "type X = { [unknownKey]: string }",
    ] {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let graph = crate::indexer::lexical::capture(
            tree.root_node(),
            source.as_bytes(),
            "ts",
            &mut vec![],
            &[],
            crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
        )
        .unwrap();
        let declaration = tree.root_node().named_child(0).unwrap();
        let capture = Capture {
            source: source.as_bytes(),
            syntax: &crate::languages::typescript::flow::TS_LEXICAL_SYNTAX,
            graph: &graph,
            anchors: Default::default(),
        };
        assert!(
            matches!(
                capture.expr(declaration.child_by_field_name("value").unwrap()),
                TypeExpr::Unknown
            ),
            "{source}"
        );
    }
}
