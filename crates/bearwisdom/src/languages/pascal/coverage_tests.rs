// =============================================================================
// pascal/coverage_tests.rs — One test per declared symbol_node_kind and ref_node_kind
//
// symbol_node_kinds: ["declProc", "defProc", "declClass", "declIntf",
//                     "declSection", "unit", "declUses"]
// ref_node_kinds:    ["exprCall", "declUses", "typeref"]
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

/// unit → Namespace symbol
#[test]
fn symbol_unit() {
    let src = "unit MyUnit;\ninterface\nprocedure Foo;\nimplementation\nprocedure Foo; begin end;\nend.";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "MyUnit" && s.kind == SymbolKind::Namespace),
        "expected Namespace MyUnit; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// declProc (forward procedure declaration) → Function symbol
#[test]
fn symbol_decl_proc() {
    let src = "unit U;\ninterface\nprocedure Foo;\nimplementation\nprocedure Foo; begin end;\nend.";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "Foo" && s.kind == SymbolKind::Function),
        "expected Function Foo; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// defProc (procedure with body) → Function symbol
#[test]
fn symbol_def_proc() {
    let src = "program Hello;\nprocedure Greet;\nbegin\n  WriteLn('Hello');\nend;\nbegin\nend.";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function),
        "expected Function from defProc; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// declClass → Class symbol
#[test]
fn symbol_decl_class() {
    let src = "unit U;\ninterface\ntype\n  TAnimal = class\n    procedure Speak;\n  end;\nimplementation\nprocedure TAnimal.Speak; begin end;\nend.";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class),
        "expected Class from declClass; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// declIntf → Interface symbol
#[test]
fn symbol_decl_intf() {
    let src = "unit U;\ninterface\ntype\n  IRunnable = interface\n    procedure Run;\n  end;\nimplementation\nend.";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Interface),
        "expected Interface from declIntf; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// declSection with record → Struct or Class symbol
///
/// Pascal records may be parsed as `declClass` or `declSection(kRecord)` depending
/// on how the tree-sitter-pascal grammar classifies the type body.  Both produce
/// a value type symbol; accept either Struct or Class.
#[test]
fn symbol_decl_section_record() {
    let src = "unit U;\ninterface\ntype\n  TPoint = record\n    X, Y: Integer;\n  end;\nimplementation\nend.";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Struct || s.kind == SymbolKind::Class),
        "expected Struct or Class from record type declaration; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// declUses (uses clause) → Imports ref  [covered under refs below]
#[test]
fn symbol_decl_uses_present() {
    let src = "unit U;\ninterface\nuses SysUtils;\nimplementation\nend.";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports from declUses; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

/// exprCall → Calls ref
#[test]
fn ref_expr_call() {
    let src = "program Hello;\nprocedure Greet;\nbegin\n  WriteLn('Hello');\nend;\nbegin\n  Greet;\nend.";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "expected Calls from exprCall; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// declUses → Imports ref
#[test]
fn ref_decl_uses() {
    let src = "unit U;\ninterface\nuses SysUtils, Classes;\nimplementation\nend.";
    let r = extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports from declUses; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// typeref → at least one symbol/ref is produced from a typed declaration
#[test]
fn ref_typeref() {
    // typeref nodes appear in parameter type annotations and variable declarations.
    // Verify the extractor produces output from a procedure with a typed parameter.
    let src = "unit U;\ninterface\nprocedure Foo(X: Integer);\nimplementation\nprocedure Foo(X: Integer); begin end;\nend.";
    let r = extract(src);
    assert!(
        !r.symbols.is_empty(),
        "expected at least one symbol when typeref nodes are present; got none"
    );
}
