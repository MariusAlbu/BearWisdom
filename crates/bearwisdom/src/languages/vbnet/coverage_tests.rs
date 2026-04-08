// =============================================================================
// vbnet/coverage_tests.rs  —  One test per declared symbol_node_kind and ref_node_kind
//
// NOTE: The tree-sitter-vb-dotnet grammar encodes `Inherits BaseClass` inside a
// class body as field_declaration + ERROR rather than inherits_clause. The
// extractor handles this via the field_declaration arm.
//
// `Dim x As New Type()` parses the `New` expression into an as_clause with ERROR,
// so new_expression is not emitted by the grammar. The test documents this.
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
/// The grammar encodes `Inherits Animal` as field_declaration + ERROR; the
/// extractor detects this pattern via the field_declaration arm.
#[test]
fn ref_inherits_clause() {
    let r = extract("Public Class Dog\n    Inherits Animal\nEnd Class");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Inherits && rf.target_name == "Animal"),
        "expected Inherits Animal from Inherits clause; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `implements_clause`  →  Inherits edge
/// The grammar encodes `Implements IRunnable` similarly to Inherits — the
/// extractor's `inherits_base_from_field_decl` matches on "Implements" too.
#[test]
fn ref_implements_clause() {
    let r = extract("Public Class Runner\n    Implements IRunnable\nEnd Class");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Inherits && rf.target_name == "IRunnable"),
        "expected Inherits (Implements) IRunnable from Implements clause; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Rules entries not yet handled by the extractor — documented as TODO
// ---------------------------------------------------------------------------

/// symbol_node_kind: `enum_member`  →  EnumMember
/// Rules specify EnumMember for enum value entries; the extractor does not
/// currently descend into enum_block to extract individual members.
// TODO: extractor does not emit EnumMember for VB.NET enum_block children
#[test]
fn symbol_enum_member() {
    let r = extract("Public Enum Status\n  Active\n  Inactive\nEnd Enum");
    // Best-effort: at minimum the enum itself is extracted.
    assert!(
        r.symbols.iter().any(|s| s.name == "Status" && s.kind == SymbolKind::Enum),
        "expected Enum Status as prerequisite; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    // TODO: assert EnumMember "Active" once extractor handles enum_block children.
}

/// symbol_node_kind: `constructor_declaration`  →  Constructor
/// VB.NET `Sub New` is the constructor. The grammar emits a `constructor_declaration`
/// node, but the extractor only matches `method_declaration` and does not have a
/// dedicated `constructor_declaration` arm — so `Sub New` is not extracted at all.
// TODO: extractor does not match constructor_declaration — Sub New produces no symbol
#[test]
fn symbol_constructor_declaration() {
    let r = extract("Public Class Widget\n  Public Sub New()\n  End Sub\nEnd Class");
    // No assertion — just verify no panic.
    let _ = r;
}

/// symbol_node_kind: `field_declaration`  →  Field
/// The extractor only uses field_declaration to detect Inherits/Implements patterns.
/// Ordinary field declarations are not extracted as Field symbols.
// TODO: extractor does not emit Field for field_declaration
#[test]
fn symbol_field_declaration() {
    let r = extract(
        "Public Class Config\n  Private _timeout As Integer\nEnd Class",
    );
    // No assertion on Field — just verify no panic.
    let _ = r;
}

/// symbol_node_kind: `const_declaration`  →  Variable
/// The extractor has no arm for const_declaration; it falls through to walk_children.
// TODO: extractor does not emit Variable for const_declaration
#[test]
fn symbol_const_declaration() {
    let r = extract(
        "Module M\n  Const MAX_RETRY As Integer = 3\nEnd Module",
    );
    // No assertion — just verify no panic.
    let _ = r;
}

/// symbol_node_kind: `delegate_declaration`  →  Delegate
/// No arm for delegate_declaration in the extractor.
// TODO: extractor does not emit Delegate for delegate_declaration
#[test]
fn symbol_delegate_declaration() {
    let r = extract("Delegate Function Transformer(x As Integer) As Integer");
    let _ = r;
}

/// symbol_node_kind: `event_declaration`  →  Event
/// No arm for event_declaration in the extractor.
// TODO: extractor does not emit Event for event_declaration
#[test]
fn symbol_event_declaration() {
    let r = extract(
        "Public Class Button\n  Public Event Clicked As EventHandler\nEnd Class",
    );
    let _ = r;
}
