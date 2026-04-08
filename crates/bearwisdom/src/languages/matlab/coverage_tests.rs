// =============================================================================
// matlab/coverage_tests.rs  —  One test per declared symbol_node_kind and ref_node_kind
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// Symbol node kinds
// ---------------------------------------------------------------------------

/// symbol_node_kind: `function_definition`  →  Function
#[test]
fn symbol_function_definition() {
    let r = extract("function y = foo(x)\ny = x + 1;\nend");
    assert!(
        r.symbols.iter().any(|s| s.name == "foo" && s.kind == SymbolKind::Function),
        "expected Function foo; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `class_definition`  →  Class
#[test]
fn symbol_class_definition() {
    let r = extract(
        "classdef Animal\n  methods\n    function speak(obj)\n      disp('hello');\n    end\n  end\nend",
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Animal" && s.kind == SymbolKind::Class),
        "expected Class Animal; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `assignment`  →  Variable (top-level)
#[test]
fn symbol_assignment_top_level() {
    let r = extract("threshold = 42;");
    assert!(
        r.symbols.iter().any(|s| s.name == "threshold" && s.kind == SymbolKind::Variable),
        "expected Variable threshold from top-level assignment; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Ref node kinds
// ---------------------------------------------------------------------------

/// ref_node_kind: `function_call`  →  Calls edge
#[test]
fn ref_function_call() {
    let r = extract("function y = foo(x)\ny = bar(x);\nend");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "bar" && rf.kind == EdgeKind::Calls),
        "expected Calls bar; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `field_expression` (callee)  —  obj.method() call.
/// target_name should be the method; module should be Some(object).
#[test]
fn ref_field_expression_method_call() {
    let r = extract("model.predict(X)");
    let rf = r
        .refs
        .iter()
        .find(|rf| rf.target_name == "predict" && rf.kind == EdgeKind::Calls);
    assert!(
        rf.is_some(),
        "expected Calls predict; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert_eq!(
        rf.unwrap().module.as_deref(),
        Some("model"),
        "expected module=Some(\"model\"); got {:?}",
        rf.unwrap().module
    );
}

/// ref_node_kind: `field_expression` (callee) with package prefix  —  pkg.fn() call.
#[test]
fn ref_field_expression_pkg_call() {
    let r = extract("pkg.helper(a, b)");
    let rf = r
        .refs
        .iter()
        .find(|rf| rf.target_name == "helper" && rf.kind == EdgeKind::Calls);
    assert!(
        rf.is_some(),
        "expected Calls helper; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert_eq!(
        rf.unwrap().module.as_deref(),
        Some("pkg"),
        "expected module=Some(\"pkg\"); got {:?}",
        rf.unwrap().module
    );
}

/// ref_node_kind: `postfix_operator`  —  matrix transpose and similar postfix ops.
/// The extractor lists this as a ref_node_kind but the walk does not have an
/// explicit match arm for it (the `_` fallback recurses). No ref is emitted.
/// This test documents the current behaviour — no panic, symbols extracted normally.
#[test]
fn ref_postfix_operator() {
    // `A'` is a matrix transpose using a postfix operator in MATLAB.
    let r = extract("function B = transpose_it(A)\nB = A';\nend");
    assert!(
        r.symbols.iter().any(|s| s.name == "transpose_it"),
        "expected Function transpose_it; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}
