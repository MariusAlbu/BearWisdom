use super::*;

#[test]
fn incomplete_assignment_query_cannot_mutate_existing_flow_metadata() {
    let source = "const value = make();";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let cfg = FlowConfig {
        strategy_prefix: "ts",
        assignment_query: "(identifier) @rhs",
        type_guard_query: "",
        discriminant_guard_query: "",
        type_args_query: "",
        literal_type_kinds: &[],
    };
    let mut symbols = Vec::new();
    let mut meta = FlowMeta::default();
    meta.flow_binding_lhs.insert(9, 4);
    run_assignment_query(
        &tree.root_node(),
        source.as_bytes(),
        &cfg,
        &mut symbols,
        &[],
        &mut meta,
        BindingSymbols::Synthesize,
    );
    assert!(symbols.is_empty());
    assert_eq!(meta.flow_binding_lhs.len(), 1);
    assert_eq!(meta.flow_binding_lhs.get(&9), Some(&4));
    assert!(meta.flow_binding_decl_type.is_empty());
}
