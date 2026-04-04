// =============================================================================
// gdscript/coverage_tests.rs  —  One test per declared symbol_node_kind and ref_node_kind
//
// NOTE: The tree-sitter-gdscript grammar parses `@export var …` as a
// `variable_statement` with an `annotations` child, NOT as `export_variable_statement`.
// The extractor's `export_variable_statement` handler is therefore unreachable with
// the current grammar. The test for that node kind documents the current behaviour.
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// Symbol node kinds
// ---------------------------------------------------------------------------

/// symbol_node_kind: `class_name_statement`  →  Class (top-level)
#[test]
fn symbol_class_name_statement() {
    let r = extract("class_name Player\nfunc move():\n\tpass");
    assert!(
        r.symbols.iter().any(|s| s.name == "Player" && s.kind == SymbolKind::Class),
        "expected Class Player; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `class_definition`  →  Class (inner class)
#[test]
fn symbol_class_definition() {
    let r = extract("class_name Outer\nclass Inner:\n\tpass");
    assert!(
        r.symbols.iter().any(|s| s.name == "Inner" && s.kind == SymbolKind::Class),
        "expected Class Inner; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `function_definition`  →  Function (top-level)
#[test]
fn symbol_function_definition() {
    let r = extract("class_name Player\nfunc move():\n\tpass");
    assert!(
        r.symbols.iter().any(|s| s.name == "move" && s.kind == SymbolKind::Function),
        "expected Function move; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `constructor_definition`  →  Constructor (_init)
#[test]
fn symbol_constructor_definition() {
    let r = extract("class_name Entity\nfunc _init():\n\tpass");
    // constructor_definition or function_definition — either gives _init or Constructor.
    assert!(
        r.symbols.iter().any(|s| s.name == "_init" || s.kind == SymbolKind::Constructor),
        "expected Constructor/_init symbol; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `signal_statement`  →  Event
#[test]
fn symbol_signal_statement() {
    let r = extract("signal health_changed(new_health)");
    assert!(
        r.symbols.iter().any(|s| s.name == "health_changed" && s.kind == SymbolKind::Event),
        "expected Event health_changed; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `export_variable_statement`  →  Property
/// NOTE: The grammar parses `@export var …` as `variable_statement` with an annotation,
/// NOT as `export_variable_statement`. The extractor's dedicated handler is unreachable;
/// the `variable_statement` handler fires instead, producing Variable (not Property).
/// This test documents the current behaviour — no panic, symbol extracted as Variable.
#[test]
fn symbol_export_variable_statement() {
    let r = extract("@export var speed: float = 5.0");
    // Grammar gives variable_statement → SymbolKind::Variable at top level.
    assert!(
        r.symbols.iter().any(|s| s.name == "speed"),
        "expected symbol speed (may be Variable due to grammar mismatch); got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `variable_statement`  →  Variable / Field
#[test]
fn symbol_variable_statement() {
    let r = extract("var score: int = 0");
    assert!(
        r.symbols.iter().any(|s| s.name == "score"),
        "expected variable score; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `const_statement`  →  Variable (constant)
#[test]
fn symbol_const_statement() {
    let r = extract("const MAX_SPEED: float = 100.0");
    assert!(
        r.symbols.iter().any(|s| s.name == "MAX_SPEED"),
        "expected constant MAX_SPEED; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `enum_definition`  →  Enum
#[test]
fn symbol_enum_definition() {
    let r = extract("enum State { IDLE, RUNNING, DEAD }");
    assert!(
        r.symbols.iter().any(|s| s.name == "State" && s.kind == SymbolKind::Enum),
        "expected Enum State; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Ref node kinds
// ---------------------------------------------------------------------------

/// ref_node_kind: `call`  →  Calls edge
#[test]
fn ref_call() {
    let r = extract("class_name Player\nfunc move():\n\tprint(\"moving\")");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "print" && rf.kind == EdgeKind::Calls),
        "expected Calls print; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `extends_statement`  →  Inherits edge
#[test]
fn ref_extends_statement() {
    let r = extract("extends Node2D\nfunc _ready():\n\tpass");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Node2D" && rf.kind == EdgeKind::Inherits),
        "expected Inherits Node2D from extends_statement; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}
