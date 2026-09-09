use super::*;

#[test]
fn dual_type_bindings_are_distinct_from_values_in_both_declaration_orders() {
    for source in [
        "interface Model {} class Model {}",
        "class Model {} interface Model {}",
    ] {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();
        let graph = super::super::capture(
            tree.root_node(),
            source.as_bytes(),
            "ts",
            &mut vec![],
            &[],
            crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
        )
        .unwrap();
        let name = graph.name_id("Model").unwrap();
        let value = graph.binding_at(0, name).unwrap();
        let ty = graph.type_binding_at(0, name).unwrap();
        assert_ne!(value, ty);
        assert_eq!(graph.dual_types[&value], ty);
        assert_eq!(graph.kinds[&value], crate::types::SymbolKind::Class);
    }
}
