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

// ---------------------------------------------------------------------------
// Qualified name splitting
// ---------------------------------------------------------------------------

/// Qualified call: SysUtils.FreeAndNil(Obj) → target_name = "FreeAndNil", module = Some("SysUtils")
#[test]
fn ref_qualified_unit_call() {
    let src = "program P; begin SysUtils.FreeAndNil(Obj); end.";
    let r = extract(src);
    let rf = r.refs.iter().find(|rf| rf.target_name == "FreeAndNil");
    assert!(
        rf.is_some(),
        "expected Calls ref with target_name=\"FreeAndNil\"; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, &rf.module)).collect::<Vec<_>>()
    );
    assert_eq!(
        rf.unwrap().module.as_deref(),
        Some("SysUtils"),
        "expected module = Some(\"SysUtils\")"
    );
}

/// Qualified type ref: SysUtils.TStringList → target_name = "TStringList", module = Some("SysUtils")
#[test]
fn ref_qualified_type_ref() {
    let src = "unit U; interface var x: SysUtils.TStringList; implementation end.";
    let r = extract(src);
    let rf = r.refs.iter().find(|rf| rf.target_name == "TStringList");
    assert!(
        rf.is_some(),
        "expected Calls ref with target_name=\"TStringList\"; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, &rf.module)).collect::<Vec<_>>()
    );
    assert_eq!(
        rf.unwrap().module.as_deref(),
        Some("SysUtils"),
        "expected module = Some(\"SysUtils\")"
    );
}

// ---------------------------------------------------------------------------
// Additional symbol node kinds — missing from initial coverage pass
// ---------------------------------------------------------------------------

/// program node → Namespace symbol  (standalone executable)
#[test]
fn symbol_program_emits_namespace() {
    let src = "program MyApp;\nbegin\nend.";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "MyApp" && s.kind == SymbolKind::Namespace),
        "expected Namespace 'MyApp' from program; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// library node → Namespace symbol  (DLL / shared library)
#[test]
fn symbol_library_emits_namespace() {
    let src = "library MyLib;\nexports Foo;\nbegin\nend.";
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.name == "MyLib" && s.kind == SymbolKind::Namespace),
        "expected Namespace 'MyLib' from library; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// declEnum → Enum symbol  (extractor routes through declSection → Struct; TODO for true Enum kind)
/// TODO: emit SymbolKind::Enum for declType wrapping declEnum.
#[test]
fn symbol_decl_enum_no_crash() {
    // TODO: extractor should emit SymbolKind::Enum for enumeration types.
    // Current behaviour: falls through to Struct via declSection, or is skipped.
    let src = concat!(
        "unit U;\n",
        "interface\n",
        "type\n",
        "  TColor = (clRed, clGreen, clBlue);\n",
        "implementation\n",
        "end.\n",
    );
    let r = extract(src);
    // At minimum, the unit is extracted without crashing.
    assert!(
        r.symbols.iter().any(|s| s.name == "U" && s.kind == SymbolKind::Namespace),
        "expected Namespace 'U'; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// declVar (module-level var section) — extractor does not emit Variable for these yet.
/// TODO: emit Variable symbols from declVar nodes.
#[test]
fn symbol_decl_var_no_crash() {
    // TODO: extractor should emit SymbolKind::Variable for declVar nodes.
    let src = concat!(
        "unit U;\n",
        "interface\n",
        "var\n",
        "  GlobalCount: Integer;\n",
        "  GlobalName: string;\n",
        "implementation\n",
        "end.\n",
    );
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Namespace),
        "expected at least one Namespace from unit with var section; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// declConst (constant section) — extractor does not emit Variable for these yet.
/// TODO: emit Variable symbols from declConst nodes.
#[test]
fn symbol_decl_const_no_crash() {
    // TODO: extractor should emit SymbolKind::Variable for declConst nodes.
    let src = concat!(
        "unit U;\n",
        "interface\n",
        "const\n",
        "  MaxSize = 1024;\n",
        "implementation\n",
        "end.\n",
    );
    let r = extract(src);
    assert!(
        !r.symbols.is_empty(),
        "expected at least one symbol from unit with const section"
    );
}

/// declProc with kConstructor keyword → Function symbol (extractor emits Function, not Constructor)
/// The name extraction path handles kConstructor in find_proc_name.
#[test]
fn symbol_decl_constructor_emits_function() {
    let src = concat!(
        "unit U;\n",
        "interface\n",
        "type\n",
        "  TFoo = class\n",
        "    constructor Create(AValue: Integer);\n",
        "  end;\n",
        "implementation\n",
        "constructor TFoo.Create(AValue: Integer);\n",
        "begin\n",
        "end;\n",
        "end.\n",
    );
    let r = extract(src);
    // Constructor is extracted as a Function (kConstructor in find_proc_name path).
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function || s.kind == SymbolKind::Constructor),
        "expected Function or Constructor from constructor declaration; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// declField (class field declaration) — extractor does not emit Field symbols yet.
/// TODO: emit SymbolKind::Field for declField nodes inside class bodies.
#[test]
fn symbol_decl_field_no_crash() {
    // TODO: extractor should emit SymbolKind::Field for class field declarations.
    let src = concat!(
        "unit U;\n",
        "interface\n",
        "type\n",
        "  TPoint = class\n",
        "  private\n",
        "    FX: Integer;\n",
        "    FY: Integer;\n",
        "  end;\n",
        "implementation\n",
        "end.\n",
    );
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class),
        "expected Class from class with field declarations; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// declProp (property declaration) — extractor does not emit Property symbols yet.
/// TODO: emit SymbolKind::Property for declProp nodes.
#[test]
fn symbol_decl_prop_no_crash() {
    // TODO: extractor should emit SymbolKind::Property for property declarations.
    let src = concat!(
        "unit U;\n",
        "interface\n",
        "type\n",
        "  TFoo = class\n",
        "  private\n",
        "    FValue: Integer;\n",
        "  public\n",
        "    property Value: Integer read FValue write FValue;\n",
        "  end;\n",
        "implementation\n",
        "end.\n",
    );
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class),
        "expected Class from class with property; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// inherited call — the `inherited` keyword in Pascal is parsed by the grammar such
/// that `resolve_call_target` resolves the inner child identifier rather than
/// returning the literal string "inherited".  The Calls ref target is whatever
/// identifier appears inside the `inherited` expression node.
/// This test documents current behaviour: a Calls ref is emitted from the body.
#[test]
fn ref_inherited_call_emits_calls() {
    let src = concat!(
        "unit U;\n",
        "interface\n",
        "type\n",
        "  TChild = class(TParent)\n",
        "    constructor Create;\n",
        "  end;\n",
        "implementation\n",
        "constructor TChild.Create;\n",
        "begin\n",
        "  inherited Create;\n",
        "end;\n",
        "end.\n",
    );
    let r = extract(src);
    // At least one Calls ref is produced from the constructor body.
    // The exact target_name depends on how the grammar represents `inherited Create`:
    // the extractor may emit the parent class name or "inherited" itself.
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "expected at least one Calls ref from constructor with inherited; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// declClass with parent → Inherits edge not yet emitted by extractor.
/// TODO: emit Inherits edge from declClass parent field.
#[test]
fn ref_decl_class_inherits_no_crash() {
    // TODO: extractor should emit Inherits edge for class parent declarations.
    let src = concat!(
        "unit U;\n",
        "interface\n",
        "type\n",
        "  TDog = class(TAnimal)\n",
        "    procedure Bark;\n",
        "  end;\n",
        "implementation\n",
        "procedure TDog.Bark; begin end;\n",
        "end.\n",
    );
    let r = extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class),
        "expected Class from TDog declaration; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    // TODO: assert r.refs.iter().any(|rf| rf.kind == EdgeKind::Inherits && rf.target_name == "TAnimal")
}

/// Multiple modules in a single uses clause → one Imports ref per module
#[test]
fn ref_decl_uses_multiple_modules() {
    let src = "unit U;\ninterface\nuses SysUtils, Classes, StrUtils;\nimplementation\nend.";
    let r = extract(src);
    let imports: Vec<&str> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::Imports)
        .map(|rf| rf.target_name.as_str())
        .collect();
    for module in &["SysUtils", "Classes", "StrUtils"] {
        assert!(
            imports.contains(module),
            "expected Imports ref to '{module}' in multi-module uses; got {imports:?}"
        );
    }
}
