// =============================================================================
// zig/coverage_tests.rs
//
// Node-kind coverage for ZigPlugin::symbol_node_kinds() and ref_node_kinds().
// Grammar is tree-sitter-zig; extraction uses the line scanner.
//
// symbol_node_kinds: function_declaration, variable_declaration, test_declaration
// ref_node_kinds:    call_expression, builtin_function
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_function_declaration_produces_function() {
    let r = extract::extract("fn add(a: i32, b: i32) i32 {\n    return a + b;\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "add"),
        "fn should produce Function(add); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_pub_function_produces_function() {
    let r = extract::extract("pub fn main() void {\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "main"),
        "pub fn should produce Function(main); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_struct_declaration_produces_struct() {
    let r = extract::extract("const Point = struct {\n    x: f32,\n    y: f32,\n};");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Struct && s.name == "Point"),
        "const struct should produce Struct(Point); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_test_declaration_produces_test() {
    let r = extract::extract("test \"addition\" {\n    try std.testing.expect(1 + 1 == 2);\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Test),
        "test block should produce Test symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_import_produces_imports_ref() {
    let r = extract::extract("const std = @import(\"std\");");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "@import should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
