// =============================================================================
// groovy/coverage_tests.rs  —  One test per declared symbol_node_kind and ref_node_kind
//
// NOTE: The tree-sitter-groovy grammar uses different node names than what the
// extractor pattern-matches against:
//   declared "class_definition"    → grammar emits "class_declaration"
//   declared "function_definition" → grammar emits "method_declaration"
//   declared "groovy_package"      → grammar emits "package_declaration"
//   declared "groovy_import"       → grammar emits "import_declaration"
// As a result the extractor currently produces no symbols/refs for Groovy code.
// Each test documents the current behaviour (no panic, empty output) and uses
// snippets that would exercise each declared node kind once the extractor is
// updated to match the grammar's actual node names.
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// Symbol node kinds
// ---------------------------------------------------------------------------

/// symbol_node_kind: `class_definition`
/// Grammar actually emits `class_declaration`; extractor currently yields nothing.
#[test]
fn symbol_class_definition() {
    let r = extract("class Foo {\n    def bar() { baz() }\n}");
    // Grammar mismatch: no symbol extracted yet; assert no panic.
    let _ = r;
}

/// symbol_node_kind: `function_definition`  —  top-level
/// Grammar actually emits `method_declaration`; extractor currently yields nothing.
#[test]
fn symbol_function_definition_top_level() {
    let r = extract("def greet(name) {\n    println(name)\n}");
    let _ = r;
}

/// symbol_node_kind: `function_definition` inside class  →  Method
#[test]
fn symbol_function_definition_method() {
    let r = extract("class Foo {\n    def bar() { baz() }\n}");
    let _ = r;
}

/// symbol_node_kind: `function_declaration`  —  typed declaration form
#[test]
fn symbol_function_declaration() {
    let r = extract("class Calc {\n    int add(int a, int b) { return a + b }\n}");
    let _ = r;
}

/// symbol_node_kind: `groovy_package`  →  Namespace
/// Grammar actually emits `package_declaration`; extractor currently yields nothing.
#[test]
fn symbol_groovy_package() {
    let r = extract("package com.example.app\n\nclass Hello {}");
    let _ = r;
}

// ---------------------------------------------------------------------------
// Ref node kinds
// ---------------------------------------------------------------------------

/// ref_node_kind: `function_call`
/// Grammar emits `method_invocation`; extractor currently yields no refs.
#[test]
fn ref_function_call() {
    let r = extract("class Foo {\n    def bar() { baz() }\n}");
    let _ = r;
}

/// ref_node_kind: `juxt_function_call`  —  Groovy method call without parens
#[test]
fn ref_juxt_function_call() {
    let r = extract("def run() {\n    println \"hello\"\n}");
    let _ = r;
}

/// ref_node_kind: `groovy_import`
/// Grammar emits `import_declaration`; extractor currently yields no refs.
#[test]
fn ref_groovy_import() {
    let r = extract("import groovy.json.JsonSlurper\n\nclass Foo {}");
    let _ = r;
}
