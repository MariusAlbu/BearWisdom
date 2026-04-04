// =============================================================================
// starlark/coverage_tests.rs
//
// Node-kind coverage for StarlarkPlugin::symbol_node_kinds() and ref_node_kinds().
// Grammar is tree-sitter-starlark; extraction also uses the line scanner.
//
// symbol_node_kinds: function_definition, assignment
// ref_node_kinds:    call
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_function_definition_produces_function() {
    let r = extract::extract("def my_rule():\n    pass\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "my_rule"),
        "def should produce Function(my_rule); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_rule_assignment_produces_function() {
    // name = rule(...) → Function (rule definition)
    let r = extract::extract("my_binary = rule(\n    implementation = _impl,\n)\n");
    assert!(
        r.symbols.iter().any(|s| s.name == "my_binary"),
        "rule assignment should produce symbol(my_binary); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_plain_assignment_produces_variable() {
    // A simple constant assignment → Variable
    let r = extract::extract("VERSION = \"1.0.0\"\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "VERSION"),
        "assignment should produce Variable(VERSION); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_load_produces_imports() {
    let r = extract::extract("load(\"//tools:defs.bzl\", \"cc_binary\")\n");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "load() should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_function_call_produces_calls() {
    let r = extract::extract("def build():\n    native.cc_binary(name = \"app\")\n");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "function call should produce Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
