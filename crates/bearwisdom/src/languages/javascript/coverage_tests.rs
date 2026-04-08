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

// ---------------------------------------------------------------------------
// New coverage — node types from rules not yet exercised above
// ---------------------------------------------------------------------------

#[test]
fn coverage_arrow_function_symbol() {
    // `const fn = (x) => x` — arrow_function initializer → Function symbol from variable name.
    let r = extract::extract("const double = (x) => x * 2;");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "double"),
        "arrow_function in variable_declarator should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_generator_function_expression() {
    // `const gen = function* () {}` — generator_function expression initializer → Function.
    let r = extract::extract("const counter = function* () { yield 1; };");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "counter"),
        "generator_function expression in variable_declarator should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_constructor_method() {
    // method_definition named "constructor" should produce Constructor kind.
    let r = extract::extract("class Queue { constructor(size) { this.size = size; } }");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Constructor && s.name == "constructor"),
        "method_definition 'constructor' should produce Constructor symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_method_call_member_expression() {
    // `obj.method()` — call_expression whose function is a member_expression → Calls ref.
    let r = extract::extract("function run() { console.log('hello'); }");
    assert!(
        r.refs.iter().any(|r| r.kind == EdgeKind::Calls && r.target_name.contains("log")),
        "member_expression method call should produce Calls ref; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_require_call_imports() {
    // `const x = require('mod')` — top-level CommonJS require → Imports edge.
    let r = extract::extract(r#"const fs = require('fs');"#);
    assert!(
        r.refs.iter().any(|r| r.kind == EdgeKind::Imports
            && (r.target_name == "fs" || r.module.as_deref() == Some("fs"))),
        "require() call should produce Imports ref; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name, &r.module)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_dynamic_import() {
    // `import('module')` — dynamic import → Imports edge.
    let r = extract::extract(r#"async function load() { const m = await import('./module'); }"#);
    assert!(
        r.refs.iter().any(|r| r.kind == EdgeKind::Imports
            && (r.target_name == "./module" || r.module.as_deref() == Some("./module"))),
        "dynamic import() should produce Imports ref; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name, &r.module)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_export_reexport_with_source() {
    // `export { Foo } from './foo'` — named re-export → Imports ref with module set.
    let r = extract::extract(r#"export { handler } from './handler';"#);
    assert!(
        r.refs.iter().any(|r| r.kind == EdgeKind::Imports && r.target_name == "handler"
            && r.module.as_deref() == Some("./handler")),
        "re-export with source should produce Imports ref with module; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name, &r.module)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_object_destructuring_variable() {
    // `const { a, b } = obj` — object destructuring → one Variable symbol per binding.
    let r = extract::extract("const { name, age } = person;");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "name"),
        "object destructuring should produce Variable symbol for 'name'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "age"),
        "object destructuring should produce Variable symbol for 'age'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_array_destructuring_variable() {
    // `const [first, second] = arr` — array destructuring → one Variable symbol per element.
    let r = extract::extract("const [head, tail] = list;");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "head"),
        "array destructuring should produce Variable symbol for 'head'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "tail"),
        "array destructuring should produce Variable symbol for 'tail'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn coverage_namespace_import() {
    // `import * as ns from 'module'` — namespace import.
    // JS extractor records this as TypeRef for the local alias.
    let r = extract::extract(r#"import * as utils from './utils';"#);
    assert!(
        r.refs.iter().any(|r| r.kind == EdgeKind::TypeRef && r.target_name == "utils"
            && r.module.as_deref() == Some("./utils")),
        "namespace_import should produce TypeRef for alias with module; got: {:?}",
        r.refs.iter().map(|r| (r.kind, &r.target_name, &r.module)).collect::<Vec<_>>()
    );
}
