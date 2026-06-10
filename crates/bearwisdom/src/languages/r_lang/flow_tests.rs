// =============================================================================
// r_lang/flow_tests.rs — unit tests for r_lang/flow.rs
// =============================================================================

use super::flow::R_FLOW_CONFIG;
use tree_sitter::{Language, Query};

fn r_language() -> Language {
    tree_sitter_r::LANGUAGE.into()
}

/// The assignment query must parse without error against the R grammar.
#[test]
fn assignment_query_is_valid() {
    let lang = r_language();
    let result = Query::new(&lang, R_FLOW_CONFIG.assignment_query);
    assert!(
        result.is_ok(),
        "assignment_query parse error: {:?}",
        result.err()
    );
}

/// The config exposes both required capture names.
#[test]
fn assignment_query_has_lhs_and_rhs_captures() {
    let lang = r_language();
    let query = Query::new(&lang, R_FLOW_CONFIG.assignment_query).unwrap();
    let names = query.capture_names();
    assert!(names.iter().any(|&n| n == "lhs"), "missing @lhs capture");
    assert!(names.iter().any(|&n| n == "rhs"), "missing @rhs capture");
}

/// Empty type-guard and type-args queries are valid (the runner skips them).
#[test]
fn optional_queries_are_empty() {
    assert!(R_FLOW_CONFIG.type_guard_query.trim().is_empty());
    assert!(R_FLOW_CONFIG.type_args_query.trim().is_empty());
}

/// Strategy prefix identifies the language for tracing/logging.
#[test]
fn strategy_prefix_is_r() {
    assert_eq!(R_FLOW_CONFIG.strategy_prefix, "r");
}
