use super::*;
use crate::indexer::flow::{
    run_flow_queries, run_flow_queries_with_plugin, run_flow_queries_with_tree,
    run_flow_queries_with_tree_and_plugin, FlowConfig, MAX_FLOW_SOURCE_BYTES,
};

#[test]
fn compatibility_flow_entrypoints_recover_plugin_owned_identity_syntax() {
    let source = "function f() { const value = 1; }";
    let grammar = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let config = &crate::languages::typescript::flow::TS_FLOW_CONFIG;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).unwrap();
    let tree = parser.parse(source, None).unwrap();

    for meta in [
        run_flow_queries(
            source,
            &grammar,
            config,
            &mut vec![],
            &mut [],
            BindingSymbols::CorrelateOnly,
        ),
        run_flow_queries_with_tree(
            source,
            config,
            &mut vec![],
            &mut [],
            &tree,
            BindingSymbols::CorrelateOnly,
        ),
    ] {
        assert!(meta.lexical.is_some());
    }
}

#[test]
fn compatibility_flow_entrypoints_fail_closed_without_a_registered_owner() {
    let source = "const value = factory();";
    let grammar = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let cfg = FlowConfig {
        strategy_prefix: "unowned-flow",
        assignment_query: "",
        type_guard_query: "",
        discriminant_guard_query: "",
        type_args_query: "",
        literal_type_kinds: &[],
    };
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).unwrap();
    let tree = parser.parse(source, None).unwrap();

    for meta in [
        run_flow_queries(
            source,
            &grammar,
            &cfg,
            &mut vec![],
            &mut [],
            BindingSymbols::CorrelateOnly,
        ),
        run_flow_queries_with_tree(
            source,
            &cfg,
            &mut vec![],
            &mut [],
            &tree,
            BindingSymbols::CorrelateOnly,
        ),
    ] {
        assert!(meta.lexical.is_none());
        assert!(meta.cfg.is_empty());
        assert!(meta.flow_binding_lhs.is_empty());
    }
}

#[test]
fn oversized_sources_keep_binding_recipes_without_running_flow_queries() {
    let short = "interface Catalog<T> { first(): T; } function run() { const value = 1; }";
    let source = format!("{}{short}", " ".repeat(MAX_FLOW_SOURCE_BYTES + 1));
    let grammar = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).unwrap();
    let tree = parser.parse(&source, None).unwrap();
    let config = &crate::languages::typescript::flow::TS_FLOW_CONFIG;
    let plugin = crate::languages::typescript::TypeScriptPlugin;
    assert!(
        !run_flow_queries_with_plugin(
            short,
            &grammar,
            config,
            Some(&plugin),
            &mut vec![],
            &mut [],
            BindingSymbols::CorrelateOnly
        )
        .cfg
        .is_empty(),
        "control must exercise the optional CFG path on small sources"
    );
    for meta in [
        run_flow_queries_with_plugin(
            &source,
            &grammar,
            config,
            Some(&plugin),
            &mut vec![],
            &mut [],
            BindingSymbols::CorrelateOnly,
        ),
        run_flow_queries_with_tree_and_plugin(
            &source,
            config,
            Some(&plugin),
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
