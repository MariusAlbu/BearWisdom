// =============================================================================
// r_lang/coverage_tests.rs — Node-kind coverage tests for the R extractor
//
// symbol_node_kinds:
//   binary_operator, call
//
// ref_node_kinds:
//   call, namespace_operator
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

/// binary_operator (<-) with function_definition RHS → SymbolKind::Function
#[test]
fn cov_binary_operator_function_assignment_emits_function() {
    let r = extract::extract("foo <- function(x) x + 1\n");
    let sym = r.symbols.iter().find(|s| s.name == "foo");
    assert!(sym.is_some(), "expected Function 'foo'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

/// binary_operator (<-) with scalar RHS → SymbolKind::Variable
#[test]
fn cov_binary_operator_scalar_emits_variable() {
    let r = extract::extract("x <- 42\n");
    let sym = r.symbols.iter().find(|s| s.name == "x");
    assert!(sym.is_some(), "expected Variable 'x'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

/// binary_operator (<-) with R6Class call → SymbolKind::Class
#[test]
fn cov_binary_operator_r6class_emits_class() {
    let src = "Animal <- R6Class(\"Animal\", public = list(speak = function() {}))\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Class);
    assert!(sym.is_some(), "expected Class symbol from R6Class; got: {:?}", r.symbols);
}

/// call node (library) → EdgeKind::Imports
/// The extractor uses get_first_string_arg, so the package name must be a quoted
/// string literal, not a bare identifier.
#[test]
fn cov_call_library_emits_imports() {
    let r = extract::extract("library(\"ggplot2\")\n");
    let imports: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        imports.contains(&"ggplot2"),
        "expected Imports ref to 'ggplot2' from library(\"ggplot2\"); got: {imports:?}"
    );
}

/// call node (setMethod) → SymbolKind::Method
#[test]
fn cov_call_set_method_emits_method() {
    let src = "setMethod(\"show\", \"MyClass\", function(object) cat(\"hello\"))\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Method);
    assert!(sym.is_some(), "expected Method symbol from setMethod(); got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

/// call → EdgeKind::Calls  (generic call)
#[test]
fn cov_call_generic_emits_calls_ref() {
    let r = extract::extract("result <- plot(x, y)\n");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(calls.contains(&"plot"), "expected Calls ref to 'plot'; got: {calls:?}");
}

/// namespace_operator (pkg::fn) → EdgeKind::Calls, target_name = function, module = package
#[test]
fn cov_namespace_operator_emits_calls_ref() {
    let r = extract::extract("x <- dplyr::filter(df, col > 0)\n");
    let calls: Vec<_> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls && r.target_name == "filter")
        .collect();
    assert!(
        !calls.is_empty(),
        "expected Calls ref with target_name='filter' from namespace_operator; got: {:?}",
        r.refs.iter().map(|r| (&r.target_name, r.kind, &r.module)).collect::<Vec<_>>()
    );
    assert_eq!(
        calls[0].module.as_deref(),
        Some("dplyr"),
        "expected module=Some(\"dplyr\"); got: {:?}",
        calls[0].module
    );
}

/// namespace_operator (::) — target_name is the function (rhs), module is the package (lhs)
#[test]
fn ref_namespace_operator_qualified() {
    let r = extract::extract("result <- dplyr::mutate(df, x = 1)\n");
    let calls: Vec<_> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls && r.target_name == "mutate")
        .collect();
    assert!(
        !calls.is_empty(),
        "expected Calls ref with target_name='mutate'; got: {:?}",
        r.refs.iter().map(|r| (&r.target_name, r.kind, &r.module)).collect::<Vec<_>>()
    );
    assert_eq!(
        calls[0].module.as_deref(),
        Some("dplyr"),
        "expected module=Some(\"dplyr\"); got: {:?}",
        calls[0].module
    );
}

/// namespace_operator (:::) — internal package functions use the same lhs/rhs split
#[test]
fn ref_namespace_operator_triple_colon() {
    let r = extract::extract("rlang:::abort(\"msg\")\n");
    let calls: Vec<_> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls && r.target_name == "abort")
        .collect();
    assert!(
        !calls.is_empty(),
        "expected Calls ref with target_name='abort' from ::: operator; got: {:?}",
        r.refs.iter().map(|r| (&r.target_name, r.kind, &r.module)).collect::<Vec<_>>()
    );
    assert_eq!(
        calls[0].module.as_deref(),
        Some("rlang"),
        "expected module=Some(\"rlang\"); got: {:?}",
        calls[0].module
    );
}
