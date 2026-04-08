// =============================================================================
// lua/coverage_tests.rs — Node-kind coverage tests for the Lua extractor
//
// symbol_node_kinds:
//   function_declaration, variable_declaration, assignment_statement, field
//
// ref_node_kinds:
//   function_call
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

/// variable_declaration without initializer (`local x`) → emits Variable symbol
#[test]
fn cov_variable_declaration_no_init_emits_variable() {
    let r = extract::extract("local x");
    let sym = r.symbols.iter().find(|s| s.name == "x");
    assert!(sym.is_some(), "expected Variable 'x' from uninit variable_declaration; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
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

// ---------------------------------------------------------------------------
// local_function  →  Function (private)
// ---------------------------------------------------------------------------

/// `local function name(...)` → SymbolKind::Function
#[test]
fn cov_local_function_emits_function() {
    let r = extract::extract("local function helper() end");
    let sym = r.symbols.iter().find(|s| s.name == "helper");
    assert!(sym.is_some(), "expected Function 'helper' from local_function; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

// ---------------------------------------------------------------------------
// dot_index_expression method declaration  →  Method
// ---------------------------------------------------------------------------

/// `function Table.name(...)` → SymbolKind::Method with name = 'name'
#[test]
fn cov_function_declaration_dot_index_emits_method() {
    let r = extract::extract("function M.setup() end");
    let sym = r.symbols.iter().find(|s| s.name == "setup");
    assert!(sym.is_some(), "expected Method 'setup' from dot_index function_declaration; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Method);
}

/// `function Table:name(...)` (method_index_expression) → SymbolKind::Method
#[test]
fn cov_function_declaration_method_index_emits_method() {
    let r = extract::extract("function Animal:speak() end");
    let sym = r.symbols.iter().find(|s| s.name == "speak");
    assert!(sym.is_some(), "expected Method 'speak' from method_index function_declaration; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Method);
}

// ---------------------------------------------------------------------------
// assignment_statement table RHS  →  Class
// ---------------------------------------------------------------------------

/// `Name = {}` (global assignment to table) → SymbolKind::Class
#[test]
fn cov_assignment_statement_table_emits_class() {
    let r = extract::extract("Animal = {}");
    let sym = r.symbols.iter().find(|s| s.name == "Animal");
    assert!(sym.is_some(), "expected Class 'Animal' from global table assignment; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Class);
}

// ---------------------------------------------------------------------------
// dot_index_expression call  →  Calls
// ---------------------------------------------------------------------------

/// `Table.method(...)` call → EdgeKind::Calls with target = 'method'
#[test]
fn cov_dot_index_call_emits_calls() {
    let r = extract::extract("function run() M.save() end");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "save" && rf.kind == EdgeKind::Calls),
        "expected Calls ref to 'save' from dot_index call; got refs: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// assignment_statement with dot_index LHS + function_definition RHS  →  Method
// ---------------------------------------------------------------------------

/// `Table.name = function(...) ... end` → SymbolKind::Method
#[test]
fn cov_assignment_dot_index_function_emits_method() {
    let r = extract::extract("M.render = function(ctx) end");
    let sym = r.symbols.iter().find(|s| s.name == "render");
    assert!(sym.is_some(), "expected Method 'render' from assignment dot_index; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Method);
}

// ---------------------------------------------------------------------------
// setmetatable (Inherits pattern) — extractor limitation
// ---------------------------------------------------------------------------

/// `setmetatable(Child, {__index = Parent})` — Lua prototype inheritance.
/// The current extractor does not emit Inherits for this pattern.
/// Test verifies at minimum no panic and `setmetatable` is emitted as a Calls ref.
#[test]
fn cov_setmetatable_emits_calls() {
    let r = extract::extract(
        "Child = {}\nsetmetatable(Child, {__index = Parent})\n"
    );
    // setmetatable must produce a Calls ref (the function is called)
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "setmetatable" && rf.kind == EdgeKind::Calls),
        "expected Calls ref to 'setmetatable'; got refs: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    // setmetatable with __index should also emit an Inherits ref for the parent.
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Parent" && rf.kind == EdgeKind::Inherits),
        "expected Inherits ref to 'Parent' from setmetatable __index; got refs: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}
