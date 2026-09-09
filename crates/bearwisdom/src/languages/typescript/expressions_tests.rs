use super::*;

#[test]
fn only_named_expression_forms_map_to_declaration_visitors() {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    for (text, expected) in [
        ("function self() {}", Some(SymbolKind::Function)),
        ("function* self() {}", Some(SymbolKind::Function)),
        ("class Self {}", Some(SymbolKind::Class)),
        ("function() {}", None),
        ("() => 0", None),
        ("class {}", None),
    ] {
        let source = format!("const value = {text};");
        let tree = parser.parse(&source, None).unwrap();
        let node = tree
            .root_node()
            .named_child(0)
            .unwrap()
            .named_child(0)
            .unwrap()
            .child_by_field_name("value")
            .unwrap();
        assert_eq!(private_kind(node), expected, "{text}");
    }
}
