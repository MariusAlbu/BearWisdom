// =============================================================================
// indexer/flow_config_tests.rs — Validates that every per-language FlowConfig's
// tree-sitter queries compile against that language's grammar.
//
// Catches query-syntax errors (wrong node names, wrong field names, malformed
// S-expression) early — without this each broken query fails silently at
// runtime in `run_flow_queries`.
// =============================================================================

use crate::languages::default_registry;
use tree_sitter::Query;

fn check_plugin(lang_id: &str) {
    let plugin = default_registry().get_dedicated(lang_id)
        .unwrap_or_else(|| panic!("plugin missing for {lang_id}"));
    let Some(cfg) = plugin.flow_config() else { return };
    let grammar = plugin
        .grammar(lang_id)
        .unwrap_or_else(|| panic!("grammar missing for {lang_id}"));

    if !cfg.assignment_query.trim().is_empty() {
        Query::new(&grammar, cfg.assignment_query).unwrap_or_else(|e| {
            panic!("{lang_id} assignment_query failed to compile: {e:?}")
        });
    }
    if !cfg.type_guard_query.trim().is_empty() {
        Query::new(&grammar, cfg.type_guard_query).unwrap_or_else(|e| {
            panic!("{lang_id} type_guard_query failed to compile: {e:?}")
        });
    }
    if !cfg.type_args_query.trim().is_empty() {
        Query::new(&grammar, cfg.type_args_query).unwrap_or_else(|e| {
            panic!("{lang_id} type_args_query failed to compile: {e:?}")
        });
    }
}

#[test] fn ts_flow_queries_compile() { check_plugin("typescript"); }
#[test] fn py_flow_queries_compile() { check_plugin("python"); }
#[test] fn rust_flow_queries_compile() { check_plugin("rust"); }
#[test] fn java_flow_queries_compile() { check_plugin("java"); }
#[test] fn kotlin_flow_queries_compile() { check_plugin("kotlin"); }
#[test] fn scala_flow_queries_compile() { check_plugin("scala"); }
#[test] fn csharp_flow_queries_compile() { check_plugin("csharp"); }
#[test] fn go_flow_queries_compile() { check_plugin("go"); }
#[test] fn php_flow_queries_compile() { check_plugin("php"); }
#[test] fn ruby_flow_queries_compile() { check_plugin("ruby"); }
#[test] fn c_flow_queries_compile() { check_plugin("c"); }
