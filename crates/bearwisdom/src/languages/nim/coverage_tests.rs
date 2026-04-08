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

/// template_declaration → Function (compile-time substitution)
#[test]
fn cov_template_declaration_produces_function() {
    let r = extract::extract("template withLock(lock: Lock, body: untyped) =\n  acquire(lock)\n  body\n  release(lock)\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "withLock"),
        "template declaration should produce Function; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// macro_declaration → Function (AST-transforming macro)
#[test]
fn cov_macro_declaration_produces_function() {
    let r = extract::extract("macro dumpExpr(x: untyped): untyped =\n  result = newLit(x.repr)\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "dumpExpr"),
        "macro declaration should produce Function; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// iterator_declaration → Function (coroutine-style iterator)
#[test]
fn cov_iterator_declaration_produces_function() {
    let r = extract::extract("iterator countdown(n: int): int =\n  var i = n\n  while i >= 0:\n    yield i\n    dec i\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "countdown"),
        "iterator declaration should produce Function; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// converter_declaration → Function (implicit conversion)
#[test]
fn cov_converter_declaration_produces_function() {
    let r = extract::extract("converter toFloat(x: int): float =\n  float(x)\n");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "toFloat"),
        "converter declaration should produce Function; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// type_symbol_declaration → Struct (object_declaration child)
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

/// type_symbol_declaration → Enum (enum_declaration child)
#[test]
fn cov_type_enum_produces_enum() {
    let src = "type\n  Color = enum\n    Red, Green, Blue\n";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Enum && s.name == "Color"),
        "type enum should produce Enum; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// type_symbol_declaration → Interface (concept_declaration child)
#[test]
fn cov_type_concept_produces_interface() {
    let src = "type\n  Printable = concept x\n    print(x)\n";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Interface && s.name == "Printable"),
        "type concept should produce Interface; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// type_symbol_declaration → TypeAlias (non-object/enum/concept RHS)
#[test]
fn cov_type_alias_produces_typealias() {
    let src = "type\n  MyInt = int\n";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::TypeAlias && s.name == "MyInt"),
        "type alias should produce TypeAlias; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// type_symbol_declaration → Struct (tuple type)
#[test]
fn cov_type_tuple_produces_struct() {
    let src = "type\n  Pair = tuple\n    a: int\n    b: string\n";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Struct && s.name == "Pair"),
        "type tuple should produce Struct; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Single-line type declaration outside of a type section (`type Name = ...`)
#[test]
fn cov_single_line_type_decl_produces_symbol() {
    let r = extract::extract("type Alias = string\n");
    assert!(
        r.symbols.iter().any(|s| s.name == "Alias"),
        "single-line type decl should produce symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

/// import_statement with a single module
#[test]
fn cov_import_statement_produces_imports() {
    let r = extract::extract("import strutils\n");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "strutils"),
        "import statement should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

/// import_statement with multiple comma-separated modules
#[test]
fn cov_import_multiple_modules_produces_multiple_imports() {
    let r = extract::extract("import os, strutils, sequtils\n");
    let imports: Vec<_> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::Imports)
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(
        imports.contains(&"os") && imports.contains(&"strutils") && imports.contains(&"sequtils"),
        "multi-module import should produce one Imports ref per module; got: {:?}",
        imports
    );
}

/// import_from_statement (`from module import symbol`) → Imports with module as target
#[test]
fn cov_import_from_statement_produces_imports() {
    let r = extract::extract("from strutils import parseInt\n");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "strutils"),
        "from-import should produce Imports ref with module name; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
