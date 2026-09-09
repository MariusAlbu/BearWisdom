#[test]
fn predicates_use_source_slots_and_keep_optional_rest_receiver_and_generics() {
    let source = "type Callback = <T extends string = string>(this: object, value: T, flag?: boolean, ...rest: [number?]) => value is T;";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut symbols = vec![];
    let graph = crate::indexer::lexical::capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut symbols,
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap();
    let function = graph
        .types
        .signatures
        .iter()
        .find(|s| s.id.0.start == source.find('<').unwrap() as u32)
        .unwrap();
    let name = graph.name_id("T").unwrap();
    let binding = graph
        .type_binding_at(source.rfind('T').unwrap() as u32, name)
        .unwrap();
    assert_eq!(graph.type_parameter_sites[&binding], function.id);
    // Source input retains both generic obligations even without an extracted row.
    assert!(function.generics[0].constraint.is_some());
    assert!(function.generics[0].default.is_some());
    assert_eq!(function.syntax.parameters.len(), 4);
    assert!(function.syntax.parameters[0].receiver);
    assert!(function.syntax.parameters[2].optional);
    assert!(function.syntax.parameters[3].rest);
}
