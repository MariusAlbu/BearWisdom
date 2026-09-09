use super::*;

#[test]
fn modifier_spans_keep_cst_aliases_and_are_idempotent() {
    let source = "declare function make<T>(value: T): T; declare var value: string;";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let function = tree
        .root_node()
        .named_child(0)
        .unwrap()
        .named_child(0)
        .unwrap();
    let syntax = super::super::syntax_for("ts").unwrap();
    assert_eq!(start(function, syntax).column, 0);
    let mut graph = LexicalBindings::default();
    graph.type_parameters.insert(BindingId(1), (0, 8, 0));
    let mut symbols = vec![crate::types::ExtractedSymbol {
        name: "poison".into(),
        qualified_name: "poison".into(),
        kind: SymbolKind::Function,
        start_line: 0,
        start_col: 8,
        end_line: 0,
        end_col: 35,
        byte_offset: 8,
        visibility: None,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: vec![],
        generic_params: vec![],
    }];
    let declarations = [(function, BindingId(0), SymbolKind::Function)];
    for _ in 0..2 {
        normalize(&mut graph, &declarations, &mut symbols, syntax);
    }
    assert_eq!(symbols[0].start_col, 0);
    assert_eq!(graph.declaration_slots.get(&(0, 8)), Some(&Some(0)));
    assert_eq!(graph.type_parameters[&BindingId(1)], (0, 0, 0));
    let declaration = tree
        .root_node()
        .named_child(1)
        .unwrap()
        .named_child(0)
        .unwrap()
        .named_child(0)
        .unwrap();
    assert_eq!(
        start(declaration, syntax),
        declaration.start_position(),
        "variable patterns keep their own anchors"
    );
}
