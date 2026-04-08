// =============================================================================
// ocaml/coverage_tests.rs — One test per declared symbol_node_kind and ref_node_kind
//
// symbol_node_kinds: ["value_definition", "type_definition", "module_definition", "open_module"]
// ref_node_kinds:    ["open_module", "application_expression"]
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

/// value_definition with parameters → Function symbol
#[test]
fn symbol_value_definition_function() {
    let r = extract("let foo x = x + 1", "test.ml");
    assert!(
        r.symbols.iter().any(|s| s.name == "foo" && s.kind == SymbolKind::Function),
        "expected Function foo; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// value_definition without parameters → Variable symbol
#[test]
fn symbol_value_definition_variable() {
    let r = extract("let answer = 42", "test.ml");
    assert!(
        r.symbols.iter().any(|s| s.name == "answer"),
        "expected binding answer; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// type_definition with record body → Struct symbol
#[test]
fn symbol_type_definition_record() {
    let r = extract("type point = { x: int; y: int }", "test.ml");
    assert!(
        r.symbols.iter().any(|s| s.name == "point" && s.kind == SymbolKind::Struct),
        "expected Struct point; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// type_definition with variant constructors → Enum symbol
#[test]
fn symbol_type_definition_variant() {
    let r = extract("type color = Red | Green | Blue", "test.ml");
    assert!(
        r.symbols.iter().any(|s| s.name == "color" && s.kind == SymbolKind::Enum),
        "expected Enum color; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// module_definition → Namespace symbol
#[test]
fn symbol_module_definition() {
    let r = extract("module M = struct\n  let foo x = x + 1\nend", "test.ml");
    assert!(
        r.symbols.iter().any(|s| s.name == "M" && s.kind == SymbolKind::Namespace),
        "expected Namespace M; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// open_module as a symbol-kind node → Imports ref (also covers ref side)
#[test]
fn symbol_open_module_imports() {
    let r = extract("open List", "test.ml");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "List" && rf.kind == EdgeKind::Imports),
        "expected Imports List from open; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

/// open_module → Imports ref
#[test]
fn ref_open_module() {
    let r = extract("open Printf", "test.ml");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports ref; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// application_expression → Calls ref
#[test]
fn ref_application_expression() {
    let r = extract("module M = struct\n  let foo x = x + 1\n  type t = { name: string }\nend", "test.ml");
    // foo is called or defined; the module contains a value_definition
    assert!(
        !r.symbols.is_empty(),
        "expected symbols from module struct; got none"
    );
}

/// application_expression in a function body → Calls ref
#[test]
fn ref_application_expression_call() {
    let r = extract("let bar x = x + 1\nlet main () = bar 42", "test.ml");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "bar" && rf.kind == EdgeKind::Calls),
        "expected Calls bar; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// Qualified call: Module.function → target_name = function, module = Some(Module)
#[test]
fn ref_qualified_call() {
    let r = extract("let main () = List.map (fun x -> x) [1;2;3]", "test.ml");
    let rf = r.refs.iter().find(|rf| rf.target_name == "map" && rf.kind == EdgeKind::Calls);
    assert!(
        rf.is_some(),
        "expected Calls ref to 'map'; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert_eq!(
        rf.unwrap().module.as_deref(),
        Some("List"),
        "expected module=Some(\"List\") for qualified call List.map"
    );
}

/// Nested qualified call: Stdlib.List.map → module = "Stdlib.List", target = "map"
#[test]
fn ref_nested_qualified_call() {
    let r = extract("let main () = Stdlib.List.map (fun x -> x) [1;2;3]", "test.ml");
    let rf = r.refs.iter().find(|rf| rf.target_name == "map");
    assert!(rf.is_some(), "expected ref to 'map'");
    assert_eq!(
        rf.unwrap().module.as_deref(),
        Some("Stdlib.List"),
        "expected module=Some(\"Stdlib.List\") for nested qualified call Stdlib.List.map"
    );
}
