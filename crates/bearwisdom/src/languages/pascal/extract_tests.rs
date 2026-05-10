// =============================================================================
// pascal/extract_tests.rs — unit tests for pascal/extract.rs
// =============================================================================

use super::extract;
use crate::types::SymbolKind;

// ---------------------------------------------------------------------------
// Multiple sequential forward declarations (castle-fresh x3dnodes pattern)
// ---------------------------------------------------------------------------

#[test]
fn inc_fragment_multiple_forward_declarations() {
    // {$ifdef} guard wrapping multiple `TypeName = class;` forward declarations,
    // as seen in x3dnodes_initial_types.inc and similar files.
    let source = r#"{$ifdef read_interface}
  TX3DNodeList = class;
  TX3DNode = class;
  TAbstractGeometryNode = class;
  TSFNode = class;
{$endif read_interface}
"#;
    let result = extract(source);
    let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
    for expected in &["TX3DNodeList", "TX3DNode", "TAbstractGeometryNode", "TSFNode"] {
        assert!(
            result.symbols.iter().any(|s| &s.name == expected && s.kind == SymbolKind::Class),
            "expected class {} to be extracted; got: {:?}",
            expected,
            names
        );
    }
}

// ---------------------------------------------------------------------------
// Class extraction from .inc fragments (no 'type' keyword)
// ---------------------------------------------------------------------------

#[test]
fn inc_fragment_class_extracted_without_type_keyword() {
    // .inc files omit the 'type' keyword; tree-sitter produces an ERROR node.
    // The error-recovery path should still emit a Class symbol.
    let source = r#"
  TSoundAllocator = class
  strict private
    FMin: Cardinal;
  public
    procedure Update;
  end;
"#;
    let result = extract(source);
    assert!(
        result.symbols.iter().any(|s| s.name == "TSoundAllocator" && s.kind == SymbolKind::Class),
        "TSoundAllocator class should be extracted from .inc-style fragment; got: {:?}",
        result.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn inc_fragment_interface_extracted_without_type_keyword() {
    let source = r#"
  IJSObject = interface
  ['{ABC}']
    function GetName: string;
  end;
"#;
    let result = extract(source);
    assert!(
        result.symbols.iter().any(|s| s.name == "IJSObject" && s.kind == SymbolKind::Interface),
        "IJSObject interface should be extracted from .inc-style fragment; got: {:?}",
        result.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn class_with_type_keyword_extracted_normally() {
    let source = r#"type
  TSoundAllocator = class
  strict private
    FMin: Cardinal;
  public
    procedure Update;
  end;
"#;
    let result = extract(source);
    assert!(
        result.symbols.iter().any(|s| s.name == "TSoundAllocator" && s.kind == SymbolKind::Class),
        "TSoundAllocator should be extracted with 'type' keyword present; got: {:?}",
        result.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn inc_fragment_extracts_method_under_class() {
    let source = r#"
  TSoundAllocator = class
  public
    procedure Update;
    function GetCount: Integer;
  end;
"#;
    let result = extract(source);
    let class_sym = result.symbols.iter().find(|s| s.name == "TSoundAllocator");
    assert!(class_sym.is_some(), "class should be extracted");
    // Procedures/functions inside the class body should also appear
    assert!(
        result.symbols.iter().any(|s| s.name == "Update"),
        "method Update should be extracted; got: {:?}",
        result.symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}

#[test]
fn inc_fragment_with_preprocessor_directives() {
    // Castle Game Engine .inc files start with {$ifdef read_interface}
    // Tree-sitter strips these as preprocessor/comment nodes.
    let source = r#"{$ifdef read_interface}
  TCastleUserInterface = class(TCastleComponent)
  public
    procedure Draw;
  end;
{$endif read_interface}
"#;
    let result = extract(source);
    assert!(
        result.symbols.iter().any(|s| s.name == "TCastleUserInterface" && s.kind == SymbolKind::Class),
        "TCastleUserInterface should be extracted through preprocessor guards; got: {:?}",
        result.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}
