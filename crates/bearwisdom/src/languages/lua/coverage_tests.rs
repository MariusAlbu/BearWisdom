// =============================================================================
// lua/coverage_tests.rs — Node-kind coverage tests for the Lua extractor
//
// symbol_node_kinds:
//   function_declaration, function_definition, variable_declaration,
//   assignment_statement, field
//
// ref_node_kinds:
//   function_call, method_index_expression
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

/// function_declaration → SymbolKind::Function  (global named form)
#[test]
fn cov_function_declaration_emits_function() {
    let r = extract::extract("function foo() end");
    let sym = r.symbols.iter().find(|s| s.name == "foo");
    assert!(sym.is_some(), "expected Function 'foo'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

/// function_definition on RHS of assignment → SymbolKind::Function
#[test]
fn cov_function_definition_as_rhs_emits_function() {
    let r = extract::extract("local bar = function() end");
    let sym = r.symbols.iter().find(|s| s.name == "bar");
    assert!(sym.is_some(), "expected Function 'bar' from rhs function_definition; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

/// variable_declaration with table_constructor RHS → SymbolKind::Class
#[test]
fn cov_variable_declaration_table_emits_class() {
    let r = extract::extract("local MyModule = {}");
    let sym = r.symbols.iter().find(|s| s.name == "MyModule");
    assert!(sym.is_some(), "expected Class 'MyModule' from table variable_declaration; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Class);
}

/// assignment_statement with function_definition RHS → SymbolKind::Function
#[test]
fn cov_assignment_statement_function_emits_function() {
    let r = extract::extract("setup = function() end");
    let sym = r.symbols.iter().find(|s| s.name == "setup");
    assert!(sym.is_some(), "expected Function 'setup' from assignment_statement; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

/// field inside table_constructor → SymbolKind::Field or SymbolKind::Method
#[test]
fn cov_field_in_table_emits_field_or_method() {
    let src = "local M = { greet = function() end, count = 0 }";
    let r = extract::extract(src);
    let greet = r.symbols.iter().find(|s| s.name == "greet");
    assert!(greet.is_some(), "expected symbol 'greet' from table field; got: {:?}", r.symbols);
    // greet is a function value → Method
    assert_eq!(greet.unwrap().kind, SymbolKind::Method);
    let count = r.symbols.iter().find(|s| s.name == "count");
    assert!(count.is_some(), "expected symbol 'count' from table field; got: {:?}", r.symbols);
    assert_eq!(count.unwrap().kind, SymbolKind::Field);
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

/// function_call with require → EdgeKind::Imports
/// tree-sitter-lua 0.5 uses the `name` field for the callee of function_call.
#[test]
fn cov_function_call_require_emits_import() {
    let r = extract::extract("local M = require(\"mod\")");
    let imports: Vec<&str> = r.refs.iter()
        .filter(|rf| rf.kind == EdgeKind::Imports)
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(
        imports.contains(&"mod"),
        "expected Imports ref to 'mod' from require(); got refs: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// function_call → EdgeKind::Calls  (plain call)
#[test]
fn cov_function_call_emits_calls() {
    let src = "function foo() end\nfunction main() foo() end";
    let r = extract::extract(src);
    // Function symbols should be extracted
    let foo = r.symbols.iter().find(|s| s.name == "foo");
    assert!(foo.is_some(), "expected Function 'foo'; got: {:?}", r.symbols);
    // Calls edge should be emitted for foo()
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "foo" && rf.kind == EdgeKind::Calls),
        "expected Calls ref to 'foo'; got refs: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// method_index_expression (colon call `obj:update()`) → EdgeKind::Calls
#[test]
fn cov_method_index_expression_emits_calls() {
    let src = "function init() obj:update() end";
    let r = extract::extract(src);
    // function symbol still extracted
    let init = r.symbols.iter().find(|s| s.name == "init");
    assert!(init.is_some(), "expected Function 'init'; got: {:?}", r.symbols);
    // Method call via colon syntax should produce Calls ref
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "update" && rf.kind == EdgeKind::Calls),
        "expected Calls ref to 'update' from colon call; got refs: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}
