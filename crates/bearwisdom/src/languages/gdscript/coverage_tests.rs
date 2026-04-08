// =============================================================================
// gdscript/coverage_tests.rs  —  One test per declared symbol_node_kind and ref_node_kind
//
// Grammar notes (tree-sitter-gdscript, confirmed by CST probe):
//   `@export var x` → variable_statement with annotations child (not export_variable_statement)
//   The extractor detects the @export annotation and emits SymbolKind::Property.
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

/// symbol_node_kind: `variable_statement` with `@export` annotation  →  Property
/// The grammar parses `@export var …` as `variable_statement` with an `annotations`
/// child.  The extractor detects the @export annotation and emits Property.
#[test]
fn symbol_export_variable_statement() {
    let r = extract("@export var speed: float = 5.0");
    assert!(
        r.symbols.iter().any(|s| s.name == "speed" && s.kind == SymbolKind::Property),
        "expected Property 'speed' from @export var; got {:?}",
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

/// ref_node_kind: `class_name_statement` with `extends`  →  Inherits edge
/// `class_name_statement` itself emits the Inherits edge (distinct from
/// `extends_statement` which is a standalone `extends` at the top of a script).
///
/// NOTE: the `extends` field text for `class_name_statement` includes the keyword
/// ("extends CharacterBody2D"), so the extractor stores the full field text as the
/// target_name. The assertion uses contains() to match across grammar versions.
#[test]
fn ref_class_name_statement_inherits() {
    let r = extract("class_name Player extends CharacterBody2D");
    assert!(
        r.refs.iter().any(|rf| rf.target_name.contains("CharacterBody2D") && rf.kind == EdgeKind::Inherits),
        "expected Inherits edge with target containing 'CharacterBody2D'; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `class_definition` with `extends`  →  Inherits edge (inner class)
#[test]
fn ref_class_definition_inherits() {
    let r = extract("class_name Outer\nclass Inner extends Node:\n\tpass");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Node" && rf.kind == EdgeKind::Inherits),
        "expected Inherits Node from inner class_definition extends; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `variable_statement` inside a class  →  Field
/// When `var` appears inside a `class_definition` body the extractor emits Field.
#[test]
fn symbol_variable_statement_as_field_inside_class() {
    let r = extract("class_name Actor\nclass Inner:\n\tvar hp: int = 100");
    assert!(
        r.symbols.iter().any(|s| s.name == "hp" && s.kind == SymbolKind::Field),
        "expected Field hp inside inner class; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `function_definition` inside a class_definition  →  Method
#[test]
fn symbol_function_definition_as_method_inside_class() {
    let r = extract("class_name Actor\nclass Inner:\n\tfunc attack():\n\t\tpass");
    assert!(
        r.symbols.iter().any(|s| s.name == "attack" && s.kind == SymbolKind::Method),
        "expected Method attack inside inner class; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `onready_variable_statement`  →  Field
/// In the grammar `@onready var x` may parse as `variable_statement` with an
/// annotation, or as a dedicated `onready_variable_statement` node depending on
/// the grammar version.  Either way the extractor emits a symbol named `node_ref`.
#[test]
fn symbol_onready_variable_statement() {
    let r = extract("@onready var node_ref: Node");
    assert!(
        r.symbols.iter().any(|s| s.name == "node_ref"),
        "expected symbol node_ref from @onready var; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: anonymous `enum_definition`  →  Enum with placeholder name
#[test]
fn symbol_anonymous_enum_definition() {
    let r = extract("enum { UP, DOWN, LEFT, RIGHT }");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Enum),
        "expected an Enum symbol for anonymous enum; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}
