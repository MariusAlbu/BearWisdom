use super::*;

#[test]
fn receiver_values_have_distinct_binding_ids_and_physical_signature_slots() {
    let source = "struct A; struct B; impl A { fn f(&self) { g(self); } } impl B { fn f(&mut self) { g(self); } }";
    let mut parsed = crate::languages::rust_lang::extract::extract(source);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let mut data = crate::indexer::namespaces::capture(
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &parsed.symbols,
        &parsed.refs,
    )
    .unwrap();
    let graph = crate::indexer::namespaces::capture_locals(
        &mut data,
        tree.root_node(),
        source.as_bytes(),
        "rust",
        &mut parsed.symbols,
        &parsed.refs,
        crate::indexer::flow::BindingSymbols::Synthesize,
    );
    assert_eq!(graph.types.receiver_values.len(), 2);
    let slots: std::collections::HashSet<_> =
        graph.types.receiver_values.values().copied().collect();
    assert_eq!(slots.len(), 2);
    assert!(slots.iter().all(|s| graph.types.receivers.contains_key(s)));
}

#[test]
fn receiver_output_reuses_named_or_placeholder_region_without_counting_self_arguments() {
    for receiver in ["&'a self", "&'_ self", "self: &'a Self", "self: &'_ Self"] {
        let source = format!("struct C<'b> {{ inner: &'b u8 }} impl<'b> C<'b> {{ fn f<'a>({receiver}, other: &u8) -> &u8 {{ loop {{}} }} }}");
        let mut parsed = crate::languages::rust_lang::extract::extract(&source);
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(&source, None).unwrap();
        let mut data = crate::indexer::namespaces::capture(
            tree.root_node(),
            source.as_bytes(),
            "rust",
            &parsed.symbols,
            &parsed.refs,
        )
        .unwrap();
        let graph = crate::indexer::namespaces::capture_locals(
            &mut data,
            tree.root_node(),
            source.as_bytes(),
            "rust",
            &mut parsed.symbols,
            &parsed.refs,
            crate::indexer::flow::BindingSymbols::Synthesize,
        );
        let method = parsed.symbols.iter().position(|s| s.name == "f").unwrap();
        let TypeExpr::Output { inputs, .. } = &graph.types.returns[&method] else {
            panic!("output relationship");
        };
        assert_eq!(inputs.len(), 1);
        assert_eq!(graph.types.parameters[&method].len(), 1);
        let TypeExpr::Indirect {
            region: Some(region),
            ..
        } = &graph.types.receivers[&method]
        else {
            panic!("receiver reference");
        };
        assert_eq!(format!("{:?}", inputs[0]), format!("{:?}", region));
        if receiver.contains("'a") {
            assert!(
                matches!(region.as_ref(), TypeExpr::Parameter { owner: Some(slot), index: 0 } if *slot == method)
            );
        } else {
            assert!(
                matches!(region.as_ref(), TypeExpr::InputRegion { owner, .. } if *owner == method)
            );
        }
    }
}
