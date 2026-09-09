use super::*;

fn parse(source: &str) -> LexicalBindings {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut symbols = Vec::new();
    let mut data =
        super::super::capture(tree.root_node(), source.as_bytes(), "rust", &symbols, &[]).unwrap();
    super::super::capture_locals(
        &mut data,
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &mut symbols,
        &[],
        crate::indexer::flow::BindingSymbols::Synthesize,
    )
}

#[test]
fn match_arms_own_pattern_bindings_without_shadowing_sibling_or_outer_reads() {
    let source = "fn f(x:E, value:B) { match x { E::Item(value) => value.save(), E::Empty => value.save() } value.save(); }";
    let graph = parse(source);
    let name = graph.name_id("value").unwrap();
    let reads: Vec<_> = source
        .match_indices("value.save")
        .map(|(byte, _)| graph.binding_at(byte as u32, name))
        .collect();
    assert!(reads[0].is_some());
    assert_ne!(reads[0], reads[1]);
    assert_eq!(reads[1], reads[2]);
}

#[test]
fn ordered_local_bindings_share_scopes_without_merging_shadowed_declarations() {
    let source = "fn f() { let p = first(); let p = p.convert(); p.touch(); }";
    let graph = parse(source);
    let name = graph.name_id("p").unwrap();
    let first = graph
        .binding_at(source.find("p.convert").unwrap() as u32, name)
        .unwrap();
    let second = graph
        .binding_at(source.find("p.touch").unwrap() as u32, name)
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(
        graph.declaration_starts[&(source.find("p = p").unwrap() as u32)],
        second
    );
}

#[test]
fn local_items_do_not_capture_outer_values_but_closures_do() {
    let source = "fn f() { let p = first(); fn g() { p.touch(); } let c = || { p.touch(); }; }";
    let graph = parse(source);
    let name = graph.name_id("p").unwrap();
    assert_eq!(
        graph.binding_at(source.find("p.touch").unwrap() as u32, name),
        None
    );
    assert!(graph
        .binding_at(source.rfind("p.touch").unwrap() as u32, name)
        .is_some());
}

#[test]
fn extracted_closure_parameter_value_rows_are_adopted_by_source_address() {
    let source = "fn f() { let callback = |value| { value.touch(); }; }";
    let mut extracted = crate::languages::rust_lang::extract::extract(source);
    let parameter = extracted
        .symbols
        .iter()
        .position(|s| s.name == "value")
        .unwrap();
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut data = super::super::capture(
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &extracted.symbols,
        &extracted.refs,
    )
    .unwrap();
    let graph = super::super::capture_locals(
        &mut data,
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &mut extracted.symbols,
        &extracted.refs,
        crate::indexer::flow::BindingSymbols::Synthesize,
    );
    assert_eq!(
        extracted
            .symbols
            .iter()
            .filter(|s| s.name == "value")
            .count(),
        1
    );
    let binding = graph.symbols[&parameter];
    assert!(graph.lexical_only.contains(&binding));
    assert_eq!(graph.symbol_slots[&binding], Some(parameter));
}
