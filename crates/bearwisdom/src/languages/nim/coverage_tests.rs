// =============================================================================
// nim/coverage_tests.rs
//
// Node-kind coverage for NimPlugin::symbol_node_kinds() and ref_node_kinds().
// Grammar returns None; extraction is performed by the line scanner.
//
// symbol_node_kinds: proc_declaration, func_declaration, method_declaration,
//                   template_declaration, macro_declaration,
//                   iterator_declaration, converter_declaration,
//                   type_symbol_declaration
// ref_node_kinds:    call, dot_generic_call, import_statement,
//                   import_from_statement
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_proc_declaration_produces_function() {
    let r = extract::extract("proc foo(x: int): int =\n  x + 1\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "foo"),
        "proc declaration should produce Function; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_func_declaration_produces_function() {
    let r = extract::extract("func pure(x: int): int =\n  x * 2\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "pure"),
        "func declaration should produce Function; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_method_declaration_produces_method() {
    let r = extract::extract("method greet(self: Animal): string =\n  \"hello\"\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Method && s.name == "greet"),
        "method declaration should produce Method; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_type_object_produces_struct() {
    let src = "type\n  Point = object\n    x: int\n    y: int\n";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Struct && s.name == "Point"),
        "type object should produce Struct; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_import_statement_produces_imports() {
    let r = extract::extract("import strutils\n");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "strutils"),
        "import statement should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
