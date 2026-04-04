// =============================================================================
// odin/coverage_tests.rs
//
// Node-kind coverage for OdinPlugin::symbol_node_kinds() and ref_node_kinds().
// Grammar is tree-sitter-odin but extraction uses the line scanner.
//
// symbol_node_kinds: procedure_declaration, struct_declaration,
//                   enum_declaration, union_declaration,
//                   const_declaration, variable_declaration, import_declaration
// ref_node_kinds:    call_expression, import_declaration, using_statement
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_procedure_declaration_produces_function() {
    let r = extract::extract("main :: proc() {\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "main"),
        "proc declaration should produce Function; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_struct_declaration_produces_struct() {
    let r = extract::extract("Vec2 :: struct {\n  x: f32,\n  y: f32,\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Struct && s.name == "Vec2"),
        "struct declaration should produce Struct; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_enum_declaration_produces_enum() {
    let r = extract::extract("Direction :: enum {\n  North,\n  South,\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Enum && s.name == "Direction"),
        "enum declaration should produce Enum; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_import_declaration_produces_imports() {
    let r = extract::extract("import \"core:fmt\"");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "import declaration should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
