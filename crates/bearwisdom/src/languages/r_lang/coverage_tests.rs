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
    let r = extract::extract("foo <- function(x) x + 1\n", "test.R");
    let sym = r.symbols.iter().find(|s| s.name == "foo");
    assert!(sym.is_some(), "expected Function 'foo'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

/// binary_operator (<-) with scalar RHS → SymbolKind::Variable
#[test]
fn cov_binary_operator_scalar_emits_variable() {
    let r = extract::extract("x <- 42\n", "test.R");
    let sym = r.symbols.iter().find(|s| s.name == "x");
    assert!(sym.is_some(), "expected Variable 'x'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

/// binary_operator (<-) with R6Class call → SymbolKind::Class
#[test]
fn cov_binary_operator_r6class_emits_class() {
    let src = "Animal <- R6Class(\"Animal\", public = list(speak = function() {}))\n";
    let r = extract::extract(src, "test.R");
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Class);
    assert!(sym.is_some(), "expected Class symbol from R6Class; got: {:?}", r.symbols);
}

/// call node (library) → EdgeKind::Imports
/// The extractor uses get_first_string_arg, so the package name must be a quoted
/// string literal, not a bare identifier.
#[test]
fn cov_call_library_emits_imports() {
    let r = extract::extract("library(\"ggplot2\")\n", "test.R");
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
    let r = extract::extract(src, "test.R");
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Method);
    assert!(sym.is_some(), "expected Method symbol from setMethod(); got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

/// call → EdgeKind::Calls  (generic call)
#[test]
fn cov_call_generic_emits_calls_ref() {
    let r = extract::extract("result <- plot(x, y)\n", "test.R");
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
    let r = extract::extract("x <- dplyr::filter(df, col > 0)\n", "test.R");
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
    let r = extract::extract("result <- dplyr::mutate(df, x = 1)\n", "test.R");
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
    let r = extract::extract("rlang:::abort(\"msg\")\n", "test.R");
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

// ---------------------------------------------------------------------------
// Additional symbol node kinds — missing from initial coverage pass
// ---------------------------------------------------------------------------

/// call (setGeneric) → SymbolKind::Method
#[test]
fn cov_call_set_generic_emits_method() {
    let src = "setGeneric(\"myGeneric\", function(x, ...) standardGeneric(\"myGeneric\"))\n";
    let r = extract::extract(src, "test.R");
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Method);
    assert!(sym.is_some(), "expected Method symbol from setGeneric(); got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().name, "myGeneric");
}

/// call (setValidity) → SymbolKind::Method
#[test]
fn cov_call_set_validity_emits_method() {
    let src = "setValidity(\"MyClass\", function(object) TRUE)\n";
    let r = extract::extract(src, "test.R");
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Method);
    assert!(sym.is_some(), "expected Method symbol from setValidity(); got: {:?}", r.symbols);
}

/// call (setRefClass) → SymbolKind::Class
#[test]
fn cov_call_set_ref_class_emits_class() {
    let src = "Counter <- setRefClass(\"Counter\", fields = list(count = \"numeric\"))\n";
    let r = extract::extract(src, "test.R");
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Class);
    assert!(sym.is_some(), "expected Class symbol from setRefClass(); got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().name, "Counter");
}

/// call (setClass) → SymbolKind::Class  (S4 class definition via LHS assignment)
/// setClass only produces a Class symbol when it is the RHS of a `<-` assignment;
/// standalone setClass(...) falls through to a generic Calls ref in the extractor.
#[test]
fn cov_call_set_class_s4_emits_class() {
    // setClass must be on the RHS of an assignment for the Class symbol to be emitted.
    let src = "Person <- setClass(\"Person\", representation(name = \"character\", age = \"numeric\"))\n";
    let r = extract::extract(src, "test.R");
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Class);
    assert!(sym.is_some(), "expected Class symbol from setClass(); got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().name, "Person");
}

/// call (test_that) → SymbolKind::Test
#[test]
fn cov_call_test_that_emits_test() {
    let src = "test_that(\"adds correctly\", { expect_equal(1 + 1, 2) })\n";
    let r = extract::extract(src, "test.R");
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Test);
    assert!(sym.is_some(), "expected Test symbol from test_that(); got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().name, "adds correctly");
}

/// call (require) → EdgeKind::Imports
#[test]
fn cov_call_require_emits_imports() {
    let r = extract::extract("require(\"data.table\")\n", "test.R");
    let imp = r
        .refs
        .iter()
        .find(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "data.table");
    assert!(imp.is_some(), "expected Imports ref from require(); got: {:?}", r.refs);
}

/// call (requireNamespace) → EdgeKind::Imports
#[test]
fn cov_call_require_namespace_emits_imports() {
    let r = extract::extract("requireNamespace(\"purrr\")\n", "test.R");
    let imp = r
        .refs
        .iter()
        .find(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "purrr");
    assert!(imp.is_some(), "expected Imports ref from requireNamespace(); got: {:?}", r.refs);
}

/// binary_operator (->) right-assignment → SymbolKind::Variable
#[test]
fn cov_binary_operator_right_assign_emits_variable() {
    // `42 -> y` is right-assignment; the name bound is `y`
    let r = extract::extract("42 -> y\n", "test.R");
    let sym = r.symbols.iter().find(|s| s.name == "y");
    assert!(sym.is_some(), "expected Variable 'y' from right-assignment; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

/// binary_operator (<<-) superassignment with function RHS → SymbolKind::Function
#[test]
fn cov_binary_operator_superassign_function() {
    // `<<-` is the superassignment operator; semantically the same extraction path
    let r = extract::extract("counter <<- function() count + 1\n", "test.R");
    let sym = r.symbols.iter().find(|s| s.name == "counter");
    assert!(sym.is_some(), "expected Function 'counter' from <<- assignment; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

/// S3 method naming convention: `print.foo <- function(x, ...) {}` → Function named "print.foo"
#[test]
fn cov_binary_operator_s3_method_naming() {
    let r = extract::extract("print.myclass <- function(x, ...) cat(\"myclass\", \"\\n\")\n", "test.R");
    let sym = r.symbols.iter().find(|s| s.name == "print.myclass");
    assert!(
        sym.is_some(),
        "expected Function 'print.myclass' (S3 method); got: {:?}",
        r.symbols
    );
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

/// call (it) test framework variant → SymbolKind::Test
#[test]
fn cov_call_it_emits_test() {
    let src = "it(\"behaves correctly\", { expect_true(TRUE) })\n";
    let r = extract::extract(src, "test.R");
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Test);
    assert!(sym.is_some(), "expected Test symbol from it(); got: {:?}", r.symbols);
}

// ---------------------------------------------------------------------------
// NAMESPACE file parsing tests
// ---------------------------------------------------------------------------

/// export(func) in NAMESPACE file → SymbolKind::Function
#[test]
fn cov_namespace_export_emits_function() {
    let src = "# Generated by roxygen2\nexport(filter)\nexport(mutate)\nexport(select)\n";
    let r = extract::extract(src, "ext:r:dplyr/NAMESPACE");
    assert!(
        r.symbols.iter().any(|s| s.name == "filter"),
        "expected Function 'filter' from NAMESPACE export(); got: {:?}",
        r.symbols
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "mutate"),
        "expected Function 'mutate'; got: {:?}",
        r.symbols
    );
    assert!(r.refs.is_empty(), "NAMESPACE parser should emit no refs; got: {:?}", r.refs);
}

/// exportPattern in NAMESPACE → emits pattern as Function symbol
#[test]
fn cov_namespace_export_pattern_emits_function() {
    let src = "exportPattern(\"^[^\\\\.]\")\n";
    let r = extract::extract(src, "pkg/NAMESPACE");
    assert!(
        !r.symbols.is_empty(),
        "expected symbol from exportPattern(); got: {:?}",
        r.symbols
    );
}
