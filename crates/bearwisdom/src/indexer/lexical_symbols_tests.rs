use super::*;

#[test]
fn identical_coordinates_with_distinct_slots_are_ambiguous() {
    let mut graph = LexicalBindings::default();
    let scope = graph.add_scope(None, 0, 100, true);
    let name = graph.intern("value");
    let binding = graph.declare(scope, name, 1, None);
    graph.attach_symbol(3, binding);
    graph.attach_symbol(7, binding);
    graph.attach_symbol(3, binding);
    assert_eq!(graph.symbol_at(10, "value"), None);
}

fn ingest(source: &str, policy: BindingSymbols) -> (Vec<ExtractedSymbol>, crate::types::FlowMeta) {
    let plugin = crate::languages::default_registry()
        .get_dedicated("typescript")
        .unwrap();
    let extracted = plugin.extract(source, "fixture.ts", "typescript");
    let mut symbols = extracted.symbols;
    let mut refs = extracted.refs;
    let meta = crate::indexer::flow::run_flow_queries(
        source,
        &plugin.grammar("typescript").unwrap(),
        plugin.flow_config().unwrap(),
        &mut symbols,
        &mut refs,
        policy,
    );
    (symbols, meta)
}

#[test]
fn same_line_parameters_have_exact_slots_and_parents_and_seeds() {
    let source = "function a(cb: () => A) {} function b(cb: () => B) {}";
    let (symbols, meta) = ingest(source, BindingSymbols::Synthesize);
    let graph = meta.lexical.as_ref().unwrap();
    let positions: Vec<_> = source
        .match_indices("cb:")
        .map(|(byte, _)| byte as u32)
        .collect();
    let slots: Vec<_> = positions
        .iter()
        .map(|&byte| graph.symbol_at(byte, "cb").unwrap())
        .collect();
    assert_ne!(slots[0], slots[1]);
    for (i, expected) in ["a", "b"].iter().enumerate() {
        let row = &symbols[slots[i]];
        assert_eq!(row.kind, SymbolKind::Parameter);
        assert_eq!(row.byte_offset, positions[i]);
        assert_eq!(row.start_col, positions[i]);
        assert_eq!(symbols[row.parent_index.unwrap()].name, *expected);
        assert_eq!(
            meta.flow_binding_decl_type[&slots[i]],
            ["() => A", "() => B"][i]
        );
    }
}

#[test]
fn skipped_external_rows_and_symbol_less_files_do_not_synthesize() {
    let (symbols, meta) = ingest(
        "function f(cb: () => void) { cb(); }",
        BindingSymbols::CorrelateOnly,
    );
    assert!(symbols.iter().all(|s| s.kind != SymbolKind::Parameter));
    let graph = meta.lexical.unwrap();
    assert_eq!(graph.symbol_at(11, "cb"), None);
    let mut graph = LexicalBindings::default();
    let scope = graph.add_scope(None, 0, 20, true);
    let name = graph.intern("cb");
    let binding = graph.declare(scope, name, 1, None);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse("cb", None).unwrap();
    let node = tree
        .root_node()
        .named_child(0)
        .unwrap()
        .named_child(0)
        .unwrap();
    let mut empty = Vec::new();
    reconcile(
        &mut graph,
        &[(node, binding, SymbolKind::Variable)],
        b"cb",
        &mut empty,
        BindingSymbols::Synthesize,
    );
    assert!(empty.is_empty());
    assert!(graph.symbol_slots.is_empty());
}

#[test]
fn repeated_ingestion_keeps_slots_and_existing_property_rows_intact() {
    let source = "class C { constructor(public cb: () => void) {} }";
    let (mut symbols, first) = ingest(source, BindingSymbols::Synthesize);
    let original_rows: Vec<_> = symbols
        .iter()
        .map(|s| (s.byte_offset, s.kind, s.parent_index))
        .collect();
    let plugin = crate::languages::default_registry()
        .get_dedicated("typescript")
        .unwrap();
    let mut refs = plugin.extract(source, "fixture.ts", "typescript").refs;
    let second = crate::indexer::flow::run_flow_queries(
        source,
        &plugin.grammar("typescript").unwrap(),
        plugin.flow_config().unwrap(),
        &mut symbols,
        &mut refs,
        BindingSymbols::Synthesize,
    );
    assert_eq!(
        symbols
            .iter()
            .map(|s| (s.byte_offset, s.kind, s.parent_index))
            .collect::<Vec<_>>(),
        original_rows
    );
    assert_eq!(
        first.lexical.unwrap().symbol_slots,
        second.lexical.unwrap().symbol_slots
    );
    assert!(symbols
        .iter()
        .any(|s| s.kind == SymbolKind::Property && s.name == "cb"));
    assert!(symbols
        .iter()
        .any(|s| s.kind == SymbolKind::Parameter && s.name == "cb"));
}
