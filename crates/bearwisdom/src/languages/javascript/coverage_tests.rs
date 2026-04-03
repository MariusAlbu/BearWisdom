// =============================================================================
// javascript/coverage_tests.rs
//
// One test per node kind declared in JavascriptPlugin::symbol_node_kinds() and
// ref_node_kinds(). Each test parses a minimal snippet and asserts the expected
// Symbol or Ref is produced.
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn coverage_class_declaration() {
    let r = extract::extract("class Animal {}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name == "Animal"),
        "class_declaration should produce Class symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_class_expression() {
    // The `class` node kind (anonymous class expression assigned to a variable).
    let r = extract::extract("const MyClass = class {};");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name == "MyClass"),
        "class expression should produce Class symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_function_declaration() {
    let r = extract::extract("function greet(name) { return name; }");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "greet"),
        "function_declaration should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_generator_function_declaration() {
    let r = extract::extract("function* counter() { yield 1; }");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "counter"),
        "generator_function_declaration should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_function_expression() {
    let r = extract::extract("const add = function(a, b) { return a + b; };");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "add"),
        "function_expression should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_method_definition() {
    let r = extract::extract("class Svc { handle() {} }");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Method && s.name == "handle"),
        "method_definition should produce Method symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_variable_declaration() {
    // `var` keyword → variable_declaration node.
    let r = extract::extract("var legacyVar = 42;");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "legacyVar"),
        "variable_declaration should produce Variable symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_lexical_declaration() {
    let r = extract::extract("const apiUrl = 'http://example.com';");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "apiUrl"),
        "lexical_declaration should produce Variable symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_field_definition() {
    let r = extract::extract("class Svc { count = 0; }");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Property && s.name == "count"),
        "field_definition should produce Property symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn coverage_call_expression() {
    let r = extract::extract("function run() { fetchData(); }");
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::Calls && r.target_name == "fetchData"),
        "call_expression should produce Calls ref; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_new_expression() {
    // JS extractor uses EdgeKind::Calls for new_expression (matching existing behaviour).
    let r = extract::extract("const emitter = new EventEmitter();");
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::Calls && r.target_name == "EventEmitter"),
        "new_expression should produce Calls ref; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_import_statement() {
    let r = extract::extract(r#"import { useState } from 'react';"#);
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "useState"),
        "import_statement should produce TypeRef ref; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_export_statement() {
    // export_statement wrapping a class — the inner class should still be extracted.
    let r = extract::extract("export class Router {}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name == "Router"),
        "export_statement should pass through to inner class declaration; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_class_heritage() {
    // class_heritage contains the extends clause — base class should be captured.
    let r = extract::extract("class Dog extends Animal {}");
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::Inherits && r.target_name == "Animal"),
        "class_heritage / extends should produce Inherits ref; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_jsx_opening_element() {
    // JSX grammar is used by tree-sitter-javascript for JSX files.
    // Parse with the standard JS grammar — JSX elements produce jsx_opening_element.
    let r = extract::extract("function App() { return <Modal>hi</Modal>; }");
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::Calls && r.target_name == "Modal"),
        "jsx_opening_element should produce Calls ref for PascalCase component; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_jsx_self_closing_element() {
    let r = extract::extract("function App() { return <Button />; }");
    assert!(
        r.refs
            .iter()
            .any(|r| r.kind == EdgeKind::Calls && r.target_name == "Button"),
        "jsx_self_closing_element should produce Calls ref for PascalCase component; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}
