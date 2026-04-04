// =============================================================================
// gleam/coverage_tests.rs
//
// Node-kind coverage for GleamPlugin::symbol_node_kinds() and ref_node_kinds().
// symbol_node_kinds: function, external_function, type_definition,
//                   type_alias, constant, import
// ref_node_kinds:    function_call, import, binary_expression
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_function_produces_function_symbol() {
    let r = extract::extract("pub fn add(a, b) { a + b }");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "add"),
        "pub fn should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_private_function_produces_function_symbol() {
    let r = extract::extract("fn helper(x) { x }");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "helper"),
        "fn should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_type_definition_produces_enum() {
    // `pub type` with constructors → Enum (ADT / custom type)
    let r = extract::extract("pub type Color {\n  Red\n  Green\n  Blue\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Enum && s.name == "Color"),
        "pub type should produce Enum symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_type_alias_produces_type_alias() {
    let r = extract::extract("pub type Name = String");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::TypeAlias && s.name == "Name"),
        "pub type alias should produce TypeAlias; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_constant_produces_variable() {
    let r = extract::extract("pub const max_size = 100");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "max_size"),
        "pub const should produce Variable symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_import_produces_imports_ref() {
    let r = extract::extract("import gleam/list");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name.contains("list")),
        "import should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_pipeline_produces_calls_ref() {
    // `|>` pipeline → binary_expression → Calls edge for the function on the RHS
    let r = extract::extract("pub fn run() {\n  [1, 2] |> list.map(double)\n}");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "pipeline (|>) should produce Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
