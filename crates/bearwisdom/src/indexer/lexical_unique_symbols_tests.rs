#[test]
fn unique_annotations_retain_exact_owner_spans_without_any_navigation_rows() {
    let source = "declare const key: unique symbol; interface Keys { readonly tag: unique symbol; } declare class Factory { static readonly tag: unique symbol; }";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    assert!(!tree.root_node().has_error());
    let graph = crate::indexer::lexical::capture(
        tree.root_node(),
        source.as_bytes(),
        "ts",
        &mut vec![],
        &[],
        crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
    )
    .unwrap();
    let mut sites = graph.types.unique_symbols.clone();
    sites.sort_by_key(|span| span.start);
    let expected = [
        "key: unique symbol",
        "readonly tag: unique symbol",
        "static readonly tag: unique symbol",
    ];
    assert_eq!(sites.len(), expected.len());
    for (site, expected) in sites.iter().zip(expected) {
        assert_eq!(&source[site.start as usize..site.end as usize], expected);
    }
    let name = graph.name_id("key").unwrap();
    let binding = graph
        .reference_binding_at(source.find("key:").unwrap() as u32, name)
        .unwrap();
    assert!(
        matches!(graph.types.annotations.get(&binding), Some(super::super::TypeExpr::UniqueSymbol(site)) if *site == sites[0])
    );
}

#[test]
fn compiler_labelled_unique_symbol_owners_match_both_dialects() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../resolution_oracle/unique_symbol_fixtures.json"
    ))
    .unwrap();
    for grammar in [
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        tree_sitter_typescript::LANGUAGE_TSX,
    ] {
        for case in &cases {
            let source = case["source"].as_str().unwrap();
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&grammar.into()).unwrap();
            let tree = parser.parse(source, None).unwrap();
            assert!(!tree.root_node().has_error());
            let graph = crate::indexer::lexical::capture(
                tree.root_node(),
                source.as_bytes(),
                "ts",
                &mut vec![],
                &[],
                crate::indexer::flow_bindings::BindingSymbols::CorrelateOnly,
            )
            .unwrap();
            let actual: Vec<_> = graph.types.unique_symbols.iter().map(|s| s.start).collect();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                case["ownerStarts"],
                "{source}"
            );
        }
    }
}
