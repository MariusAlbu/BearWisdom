use super::*;
use crate::indexer::flow::{run_flow_queries, run_flow_queries_with_tree, MAX_FLOW_SOURCE_BYTES};

#[test]
fn oversized_sources_keep_binding_recipes_without_running_flow_queries() {
    let short = "interface Catalog<T> { first(): T; } function run() { const value = 1; }";
    let source = format!("{}{short}", " ".repeat(MAX_FLOW_SOURCE_BYTES + 1));
    let grammar = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).unwrap();
    let tree = parser.parse(&source, None).unwrap();
    let config = &crate::languages::typescript::flow::TS_FLOW_CONFIG;
    assert!(
        !run_flow_queries(
            short,
            &grammar,
            config,
            &mut vec![],
            &mut [],
            BindingSymbols::CorrelateOnly
        )
        .cfg
        .is_empty(),
        "control must exercise the optional CFG path on small sources"
    );
    for meta in [
        run_flow_queries(
            &source,
            &grammar,
            config,
            &mut vec![],
            &mut [],
            BindingSymbols::CorrelateOnly,
        ),
        run_flow_queries_with_tree(
            &source,
            config,
            &mut vec![],
            &mut [],
            &tree,
            BindingSymbols::CorrelateOnly,
        ),
    ] {
        let graph = meta
            .lexical
            .expect("flow query size limits must not erase source identities");
        let globals = graph.globals.as_ref().unwrap();
        assert!(globals.complete);
        assert_eq!(globals.roots.len(), 2);
        assert_eq!(globals.roots[0].parameters.len(), 1);
        assert!(meta.flow_binding_lhs.is_empty());
        assert!(meta.narrowings.is_empty());
        assert!(meta.cfg.is_empty());
    }
}
