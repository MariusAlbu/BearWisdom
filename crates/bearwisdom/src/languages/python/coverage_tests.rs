// =============================================================================
// python/coverage_tests.rs — Node-kind coverage tests for the Python extractor
//
// Every entry in `symbol_node_kinds()` and `ref_node_kinds()` must have at
// least one test here proving it produces the expected extraction output.
// =============================================================================

use super::*;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds() coverage
// ---------------------------------------------------------------------------

/// "class_definition" → SymbolKind::Class
#[test]
fn cov_class_definition_produces_class_symbol() {
    let src = "class Foo:\n    pass\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Foo");
    assert!(sym.is_some(), "expected Class symbol 'Foo', got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Class);
}

/// "function_definition" → SymbolKind::Function
#[test]
fn cov_function_definition_produces_function_symbol() {
    let src = "def bar():\n    pass\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "bar");
    assert!(sym.is_some(), "expected Function symbol 'bar', got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

/// "decorated_definition" → class/function symbol (decorator preserved)
#[test]
fn cov_decorated_definition_produces_symbol() {
    let src = "@dataclass\nclass Config:\n    pass\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Config");
    assert!(sym.is_some(), "expected symbol from decorated_definition, got: {:?}", r.symbols);
}

/// "assignment" → SymbolKind::Variable (module-level constant)
#[test]
fn cov_assignment_produces_variable_symbol() {
    let src = "MAX_SIZE = 100\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "MAX_SIZE");
    assert!(sym.is_some(), "expected Variable symbol 'MAX_SIZE', got: {:?}", r.symbols);
}

/// "type_alias_statement" → SymbolKind::TypeAlias (Python 3.12+)
#[test]
fn cov_type_alias_statement_produces_type_alias_symbol() {
    let src = "type Point = tuple[int, int]\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Point");
    assert!(sym.is_some(), "expected TypeAlias symbol 'Point', got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::TypeAlias);
}

// ---------------------------------------------------------------------------
// ref_node_kinds() coverage
// ---------------------------------------------------------------------------

/// "call" → EdgeKind::Calls
#[test]
fn cov_call_produces_calls_ref() {
    let src = "foo()\n";
    let r = extract::extract(src);
    let calls: Vec<&str> = r.refs.iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(calls.contains(&"foo"), "expected Calls ref for foo(), got: {calls:?}");
}

/// "import_statement" → EdgeKind::Imports
#[test]
fn cov_import_statement_produces_imports_ref() {
    let src = "import os\n";
    let r = extract::extract(src);
    let imports: Vec<&str> = r.refs.iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(imports.contains(&"os"), "expected Imports ref for 'os', got: {imports:?}");
}

/// "import_from_statement" → EdgeKind::Imports
#[test]
fn cov_import_from_statement_produces_imports_ref() {
    let src = "from os import path\n";
    let r = extract::extract(src);
    let imports: Vec<&str> = r.refs.iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(imports.contains(&"path"), "expected Imports ref for 'path', got: {imports:?}");
}

/// "future_import_statement" → EdgeKind::Imports
#[test]
fn cov_future_import_statement_produces_imports_ref() {
    let src = "from __future__ import annotations\n";
    let r = extract::extract(src);
    let imports: Vec<&str> = r.refs.iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        imports.contains(&"annotations") || imports.contains(&"__future__"),
        "expected Imports ref from future import, got: {imports:?}"
    );
}

/// "typed_parameter" → EdgeKind::TypeRef (non-primitive type annotation on param)
#[test]
fn cov_typed_parameter_produces_type_ref() {
    let src = "def process(user: User) -> None:\n    pass\n";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r.refs.iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"User"),
        "expected TypeRef for 'User' from typed_parameter, got: {type_refs:?}"
    );
}

/// "typed_default_parameter" → EdgeKind::TypeRef
#[test]
fn cov_typed_default_parameter_produces_type_ref() {
    let src = "def create(conn: Connection = None) -> None:\n    pass\n";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r.refs.iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Connection"),
        "expected TypeRef for 'Connection' from typed_default_parameter, got: {type_refs:?}"
    );
}

/// "type" node on annotated assignment → EdgeKind::TypeRef for non-primitive
///
/// `x: MyClass = val` — the `type` field on the assignment node is a `type`
/// node whose content is the annotation.  Primitive types (int, str, etc.) are
/// filtered intentionally; use a user-defined class name.
#[test]
fn cov_type_annotation_non_primitive_produces_type_ref() {
    let src = "x: MyClass = None\n";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r.refs.iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"MyClass"),
        "expected TypeRef for 'MyClass' from type annotation, got: {type_refs:?}"
    );
}

/// "generic_type" → EdgeKind::TypeRef (subscript type annotation like List[User])
#[test]
fn cov_generic_type_annotation_produces_type_ref() {
    let src = "items: List[User] = []\n";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r.refs.iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"List") || type_refs.contains(&"User"),
        "expected TypeRef from generic_type annotation, got: {type_refs:?}"
    );
}

/// "union_type" → EdgeKind::TypeRef (PEP 604 `X | Y` annotation)
#[test]
fn cov_union_type_annotation_produces_type_ref() {
    let src = "def handle(val: Admin | Guest) -> None:\n    pass\n";
    let r = extract::extract(src);
    let type_refs: Vec<&str> = r.refs.iter()
        .filter(|r| r.kind == EdgeKind::TypeRef)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"Admin") || type_refs.contains(&"Guest"),
        "expected TypeRef from union_type annotation, got: {type_refs:?}"
    );
}
