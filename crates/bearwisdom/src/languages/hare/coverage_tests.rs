// =============================================================================
// hare/coverage_tests.rs
//
// Node-kind coverage for HarePlugin::symbol_node_kinds() and ref_node_kinds().
// Grammar returns None; extraction is performed by the line scanner.
//
// symbol_node_kinds: function_declaration, type_declaration,
//                   const_declaration, global_declaration
// ref_node_kinds:    call_expression, use_statement
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_function_declaration_produces_function() {
    let r = extract::extract("fn main() void = {\n\treturn;\n};");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "main"),
        "fn declaration should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_export_function_produces_function() {
    let r = extract::extract("export fn greet(name: str) void = {};");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "greet"),
        "export fn should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_type_struct_produces_struct() {
    let r = extract::extract("type Point = struct { x: i32, y: i32 };");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Struct && s.name == "Point"),
        "type struct should produce Struct; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_const_declaration_produces_variable() {
    let r = extract::extract("def MAX: size = 100;");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "MAX"),
        "def (const) should produce Variable; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_use_statement_produces_imports() {
    let r = extract::extract("use fmt;");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "fmt"),
        "use statement should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
