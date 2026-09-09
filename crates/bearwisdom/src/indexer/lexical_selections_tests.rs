use super::*;

#[test]
fn type_query_selectors_belong_to_their_direct_syntax_owner_in_both_dialects() {
    for language in [
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        tree_sitter_typescript::LANGUAGE_TSX,
    ] {
        let source = "import * as api from './api'; type Key = typeof api.outer.inner.tag;";
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language.into()).unwrap();
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
        for (selector, expected) in [
            ("outer", vec!["outer"]),
            ("inner", vec!["outer", "inner"]),
            ("tag", vec!["outer", "inner", "tag"]),
        ] {
            let byte = source.rfind(selector).unwrap() as u32;
            let binding = graph.module.members[&byte];
            assert_eq!(graph.module.imports[&binding].selectors, expected);
        }
        let [(_, crate::indexer::lexical::globals::member_surface::Key::Computed { selectors, .. })] =
            graph.types.value_queries.as_slice()
        else {
            panic!("query path missing");
        };
        assert_eq!(
            *selectors,
            ["outer", "inner", "tag"].map(|name| graph.name_id(name).unwrap())
        );
    }
}

#[test]
fn selectors_keep_type_space_and_value_shadows_separate() {
    let source = "import * as api from './api'; function f(api: Other, x: api.Model) { api.create(); } function g() { api.nested.create(); }";
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
    let shadowed = source.find("api.create()").unwrap() as u32 + 4;
    assert!(!graph.module.members.contains_key(&shadowed));
    let byte = source.find("api.Model").unwrap() as u32;
    let binding = graph.module.qualified_types[&SourceSpan {
        start: byte,
        end: byte + 9,
    }];
    assert_eq!(graph.module.imports[&binding].selectors, ["Model"]);
    let called = source.rfind("create()").unwrap() as u32;
    let binding = graph.module.members[&called];
    assert_eq!(
        graph.module.imports[&binding].selectors,
        ["nested", "create"]
    );
}
