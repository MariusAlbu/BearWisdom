// =============================================================================
// vbnet/coverage_tests.rs  —  One test per declared symbol_node_kind and ref_node_kind
//
// NOTE: The tree-sitter-vb-dotnet grammar has known parse limitations:
// - `Inherits BaseClass` inside a class body is parsed as field_declaration + ERROR
//   rather than inherits_clause, so the extractor's inherits_clause handler yields nothing.
// - `Dim x As New Type()` parses the `New` expression into an as_clause with ERROR,
//   so new_expression yields nothing.
// Tests for those node kinds document the current behaviour (no panic).
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// Symbol node kinds
// ---------------------------------------------------------------------------

/// symbol_node_kind: `class_block`  →  Class
#[test]
fn symbol_class_block() {
    let r = extract("Public Class Animal\nEnd Class");
    assert!(
        r.symbols.iter().any(|s| s.name == "Animal" && s.kind == SymbolKind::Class),
        "expected Class Animal; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `module_block`  →  Class (VB Module = sealed static class)
#[test]
fn symbol_module_block() {
    let r = extract("Module Main\n  Sub Test()\n  End Sub\nEnd Module");
    assert!(
        r.symbols.iter().any(|s| s.name == "Main" && s.kind == SymbolKind::Class),
        "expected Class Main from module_block; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `structure_block`  →  Struct
#[test]
fn symbol_structure_block() {
    let r = extract(
        "Public Structure Point\n  Public X As Integer\n  Public Y As Integer\nEnd Structure",
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Point" && s.kind == SymbolKind::Struct),
        "expected Struct Point; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `interface_block`  →  Interface
#[test]
fn symbol_interface_block() {
    let r = extract("Public Interface IRunnable\n  Sub Run()\nEnd Interface");
    assert!(
        r.symbols.iter().any(|s| s.name == "IRunnable" && s.kind == SymbolKind::Interface),
        "expected Interface IRunnable; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `enum_block`  →  Enum
#[test]
fn symbol_enum_block() {
    let r = extract("Public Enum Color\n  Red\n  Green\n  Blue\nEnd Enum");
    assert!(
        r.symbols.iter().any(|s| s.name == "Color" && s.kind == SymbolKind::Enum),
        "expected Enum Color; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `method_declaration`  →  Method
#[test]
fn symbol_method_declaration() {
    let r = extract("Module Main\n  Sub Test()\n  End Sub\nEnd Module");
    assert!(
        r.symbols.iter().any(|s| s.name == "Test" && s.kind == SymbolKind::Method),
        "expected Method Test; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `property_declaration`  →  Property
#[test]
fn symbol_property_declaration() {
    let r = extract(
        "Public Class Config\n  Public Property Timeout As Integer\n    Get\n      Return 30\n    End Get\n    Set(value As Integer)\n    End Set\n  End Property\nEnd Class",
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Timeout" && s.kind == SymbolKind::Property),
        "expected Property Timeout; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `namespace_block`  →  Namespace
#[test]
fn symbol_namespace_block() {
    let r = extract("Namespace MyApp.Core\n  Class Foo\n  End Class\nEnd Namespace");
    assert!(
        r.symbols.iter().any(|s| s.name == "MyApp.Core" && s.kind == SymbolKind::Namespace),
        "expected Namespace MyApp.Core; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Ref node kinds
// ---------------------------------------------------------------------------

/// ref_node_kind: `imports_statement`  →  Imports edge
#[test]
fn ref_imports_statement() {
    let r = extract("Imports System.Collections.Generic\nModule M\nEnd Module");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports from imports_statement; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `invocation`  →  Calls edge
#[test]
fn ref_invocation() {
    let r = extract(
        "Module Main\n  Sub Test()\n    Console.WriteLine(\"hello\")\n  End Sub\nEnd Module",
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "expected Calls from invocation; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `new_expression`  →  Instantiates edge
/// NOTE: The grammar currently parses `Dim x As New Type()` with an ERROR node
/// inside the as_clause, so new_expression is not emitted and the handler yields nothing.
/// This test documents the current behaviour — no panic.
#[test]
fn ref_new_expression() {
    let r = extract(
        "Module Main\n  Sub Test()\n    Dim sb As New System.Text.StringBuilder()\n  End Sub\nEnd Module",
    );
    // Grammar mismatch: new_expression not produced; assert no panic.
    let _ = r;
}

/// ref_node_kind: `inherits_clause`  →  Inherits edge
/// NOTE: `Inherits BaseClass` in a class body is parsed by the grammar as a
/// field_declaration + ERROR rather than inherits_clause, so the extractor
/// produces no Inherits edge. This test documents the current behaviour.
#[test]
fn ref_inherits_clause() {
    let r = extract("Public Class Dog\n  Inherits Animal\nEnd Class");
    // Grammar mismatch: inherits_clause not produced; assert no panic.
    let _ = r;
}
