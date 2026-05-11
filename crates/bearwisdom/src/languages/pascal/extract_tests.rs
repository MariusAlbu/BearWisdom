// =============================================================================
// pascal/extract_tests.rs — unit tests for pascal/extract.rs
// =============================================================================

use super::extract;
use crate::types::SymbolKind;

// ---------------------------------------------------------------------------
// Conditional type-keyword normalisation
//
// `{$ifdef FPC}object{$else}record{$endif}` leaves both `object` and `record`
// in tree-sitter's token stream, causing cascading parse errors that wipe out
// earlier type declarations.  The pre-parser normaliser collapses such spans to
// a single keyword before handing the source to tree-sitter.
// ---------------------------------------------------------------------------

#[test]
fn ifdef_type_keyword_both_branches_normalised() {
    // Pattern from kraft.pas / castleinternalglib2.pas:
    // type keyword is entirely inside a conditional — both branches contain a
    // valid Pascal type keyword (object or record).
    let src = r#"unit kraft;
interface
type TKraftForceMode=(kfmForce,kfmAcceleration);
     PKraftForceMode=^TKraftForceMode;

     TKraftInt32={$if declared(Int32)}Int32{$else}LongInt{$ifend};
     PKraftInt32=^TKraftInt32;

     TKraftVector3=record
      case integer of
       0:(x,y,z:single);
       1:(xyz:array[0..2] of single);
     end;
     PKraftVector3=^TKraftVector3;

     TQuickHullFaceList={$ifdef FPC}object{$else}record{$endif}
      Head: pointer;
      Tail: pointer;
     end;
     PQuickHullFaceList=^TQuickHullFaceList;

implementation
end.
"#;
    let result = extract(src);
    let names: Vec<(&str, SymbolKind)> = result.symbols.iter().map(|s| (s.name.as_str(), s.kind)).collect();
    // The {$ifdef FPC}object{$else}record{$endif} must not cascade-wipe earlier types.
    for expected in &["TKraftForceMode", "TKraftInt32", "TKraftVector3", "TQuickHullFaceList"] {
        assert!(
            result.symbols.iter().any(|s| &s.name == expected),
            "expected {} to be extracted after ifdef-type-kw normalisation; got: {:?}",
            expected, names
        );
    }
}

#[test]
fn conditional_type_alias_extracted() {
    // `TypeName = {$if COND}Type1{$else}Type2{$ifend}` — conditional alias value.
    let src = r#"unit TestUnit;
interface
type
  TKraftInt32={$if declared(Int32)}Int32{$else}LongInt{$ifend};
  TKraftScalar={$ifdef KraftUseDouble}double{$else}single{$endif};
  TKraftVector3=record
   case integer of
    0:(x,y,z:single);
    1:(xyz:array[0..2] of single);
  end;
  PKraftVector3=^TKraftVector3;
implementation
end.
"#;
    let result = extract(src);
    let names: Vec<(&str, SymbolKind)> = result.symbols.iter().map(|s| (s.name.as_str(), s.kind)).collect();
    assert!(
        result.symbols.iter().any(|s| s.name == "TKraftVector3"),
        "TKraftVector3 record not extracted; got: {:?}", names
    );
    assert!(
        result.symbols.iter().any(|s| s.name == "TKraftInt32"),
        "TKraftInt32 type alias not extracted; got: {:?}", names
    );
    assert!(
        result.symbols.iter().any(|s| s.name == "TKraftScalar"),
        "TKraftScalar type alias not extracted; got: {:?}", names
    );
}

#[test]
fn kraft_pas_fundamental_types_extracted() {
    // Regression: the full kraft.pas file uses {$ifdef FPC}object{$else}record{$endif}
    // at line ~1317 which previously cascaded a parse error that wiped all type
    // declarations before it (TKraftVector3, TKraftInt32, TKraftScalar, etc.).
    use std::fs;
    let path = r"F:\Work\Projects\TestProjects\pascal-castle-fresh\src\physics\kraft\kraft.pas";
    let src = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return, // skip if test project not present
    };
    let result = extract(&src);
    let missing: Vec<&str> = ["TKraftVector3", "TKraftInt32", "TKraftScalar", "TKraftForceMode"]
        .iter()
        .filter(|&&name| !result.symbols.iter().any(|s| s.name == name))
        .copied()
        .collect();
    assert!(missing.is_empty(), "fundamental types missing from kraft.pas: {:?}", missing);
}

// ---------------------------------------------------------------------------
// Variant record with nested anonymous record (glib2 cascade pattern)
// ---------------------------------------------------------------------------

#[test]
fn variant_record_nested_anonymous_record_does_not_cascade() {
    // `TGDoubleIEEE754 = record case longint of 1 : (mpn : record ... end; );`
    // The anonymous record inside a case variant, plus the `)` on a separate line,
    // was causing a parse cascade that wiped all preceding type declarations.
    let src = r#"unit TestUnit;
interface
type
  PGSList = ^TGSList;
  TGSList = record
    data: gpointer;
  end;

  PPGDoubleIEEE754 = ^PGDoubleIEEE754;
  PGDoubleIEEE754 = ^TGDoubleIEEE754;
  TGDoubleIEEE754 = record
    case longint of
      0 : (v_double: gdouble);
      1 : (
        mpn : record
          mantissa_low: guint32;
          mantissa_high: guint20;
          biased_exponent: guint11;
          sign: guint1;
        end;

  );
  end;

  TGDir = object
    data: gpointer;
  end;

implementation
end.
"#;
    let result = extract(src);
    let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
    for expected in &["PGSList", "TGSList", "TGDoubleIEEE754", "TGDir"] {
        assert!(
            result.symbols.iter().any(|s| &s.name == expected),
            "expected {} to be extracted; got: {:?}", expected, names
        );
    }
}

#[test]
fn glib2_fundamental_types_extracted() {
    use std::fs;
    let path = r"F:\Work\Projects\TestProjects\pascal-castle-fresh\src\window\gtk\gtk3\gtk3bindings\castleinternalglib2.pas";
    let src = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return,
    };
    let result = extract(&src);
    let missing: Vec<&str> = ["PGSList", "TGSList", "PGVariant", "PGString", "PGNode", "TGArray", "TGDir"]
        .iter()
        .filter(|&&n| !result.symbols.iter().any(|s| s.name == n))
        .copied()
        .collect();
    assert!(missing.is_empty(), "glib2 types missing after cascade fix: {:?}", missing);
}

// ---------------------------------------------------------------------------
// Live file: x3dnodes_standard_core.inc — class extraction
// ---------------------------------------------------------------------------

#[test]
fn x3dnodes_standard_core_classes_extracted() {
    use std::fs;
    let path = r"F:\Work\Projects\TestProjects\pascal-castle-fresh\src\scene\x3d\x3dnodes_standard_core.inc";
    let src = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return,
    };
    let result = extract(&src);
    let missing: Vec<&str> = ["TAbstractNode", "TAbstractMetadataNode", "TAbstractChildNode", "TAbstractBindableNode"]
        .iter()
        .filter(|&&n| !result.symbols.iter().any(|s| s.name == n && s.kind == SymbolKind::Class))
        .copied()
        .collect();
    assert!(missing.is_empty(), "class symbols missing from x3dnodes_standard_core.inc: {:?}", missing);
}

// ---------------------------------------------------------------------------
// Live file: castlefields_x3dsinglefield_descendants.inc — class extraction
// ---------------------------------------------------------------------------

#[test]
fn castlefields_x3dsingle_classes_extracted() {
    use std::fs;
    let path = r"F:\Work\Projects\TestProjects\pascal-castle-fresh\src\scene\x3d\castlefields_x3dsinglefield_descendants.inc";
    let src = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return,
    };
    let result = extract(&src);
    let missing: Vec<&str> = ["TSFBitMask", "TSFBool", "TSFFloat"]
        .iter()
        .filter(|&&n| !result.symbols.iter().any(|s| s.name == n && s.kind == SymbolKind::Class))
        .copied()
        .collect();
    assert!(missing.is_empty(), "class symbols missing from castlefields inc: {:?}", missing);
}

// ---------------------------------------------------------------------------
// Multiple sequential full class definitions in .inc fragment
// ---------------------------------------------------------------------------

#[test]
fn inc_fragment_multiple_full_class_definitions_extracted() {
    // .inc file containing multiple full class bodies inside {$ifdef read_interface}.
    // Each class must be extracted; later classes must not be lost when earlier
    // class bodies contain complex field declarations (subrange set types, etc.).
    let source = r#"{$ifdef read_interface}

  { Doc comment for first class. }
  TSFBitMask = class(TX3DSingleField)
  strict private
    fFlags: set of 0..31;
    function GetFlags(i: integer): boolean;
  public
    procedure ParseValue(Lexer: TX3DLexer); override;
    destructor Destroy; override;
  end;

  { Doc comment for second class. }
  TSFBool = class(TX3DSingleField)
  public
    Value: boolean;
    constructor Create(const AName: String);
    procedure Assign(Source: TPersistent); override;
  end;

  TSFFloat = class(TX3DSingleField)
  public
    Value: single;
  end;

{$endif read_interface}
"#;
    let result = extract(source);
    let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
    for expected in &["TSFBitMask", "TSFBool", "TSFFloat"] {
        assert!(
            result.symbols.iter().any(|s| &s.name == expected && s.kind == SymbolKind::Class),
            "expected class {} to be extracted; got: {:?}", expected, names
        );
    }
}

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

// ---------------------------------------------------------------------------
// `class of` metaclass followed by the actual class declaration
// ---------------------------------------------------------------------------

#[test]
fn class_of_metaclass_followed_by_class_both_extracted() {
    // `TCastleBehaviorClass = class of TCastleBehavior;` must not prevent
    // `TCastleBehavior = class(TCastleComponent)` from being extracted.
    let source = r#"{$ifdef read_interface}
  TCastleBehaviorClass = class of TCastleBehavior;

  TCastleBehavior = class(TCastleComponent)
  strict private
    FParent: TCastleTransform;
  public
    procedure Update(const SecondsPassed: Single); virtual;
  end;
{$endif read_interface}
"#;
    let result = extract(source);
    for expected in &["TCastleBehaviorClass", "TCastleBehavior"] {
        assert!(
            result.symbols.iter().any(|s| &s.name == expected && s.kind == SymbolKind::Class),
            "expected class {} to be extracted", expected
        );
    }
}

// ---------------------------------------------------------------------------
// Live file: castletransform_behavior.inc — class extraction
// ---------------------------------------------------------------------------

#[test]
fn castletransform_behavior_classes_extracted() {
    use std::fs;
    let path = r"F:\Work\Projects\TestProjects\pascal-castle-fresh\src\transform\castletransform_behavior.inc";
    let src = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return,
    };
    let result = extract(&src);
    let missing: Vec<&str> = ["TCastleBehaviorClass", "TCastleBehavior"]
        .iter()
        .filter(|&&n| !result.symbols.iter().any(|s| s.name == n && s.kind == SymbolKind::Class))
        .copied()
        .collect();
    assert!(missing.is_empty(), "classes missing from castletransform_behavior.inc: {:?}", missing);
}

// ---------------------------------------------------------------------------
// Generic-class pair: two sequential classes with `class(<generic>)` parent
// ---------------------------------------------------------------------------

#[test]
fn inc_fragment_two_generic_classes_both_extracted() {
    // When two sequential classes both use generic parent types with `<...>`,
    // both must be extracted — a cascade failure from the first's error recovery
    // must not consume the second class's name.
    let source = r#"{$ifdef read_interface}
  TMFMatrix3f = class({$ifdef FPC}specialize{$endif} TX3DSimpleMultField<
    TMatrix3,
    TSFMatrix3f,
    TMatrix3List>)
  strict protected
    function RawItemToString(const ItemNum: Integer): String; override;
    procedure AddToList(const ItemList: TMatrix3List; const Item: TSFMatrix3f); override;
    function CreateItemBeforeParse: TSFMatrix3f; override;
  public
    procedure AssignLerp(const A: Double; Value1, Value2: TX3DField); override;
    function CanAssignLerp: boolean; override;
    class function X3DType: String; override;
    class function CreateEvent(const AParentNode: TX3DFileItem; const AName: String; const AInEvent: boolean): TX3DEvent; override;
  end;

  TMFMatrix3d = class({$ifdef FPC}specialize{$endif} TX3DSimpleMultField<
    TMatrix3Double,
    TSFMatrix3d,
    TMatrix3DoubleList>)
  strict protected
    function RawItemToString(const ItemNum: Integer): String; override;
    procedure AddToList(const ItemList: TMatrix3DoubleList; const Item: TSFMatrix3d); override;
    function CreateItemBeforeParse: TSFMatrix3d; override;
  public
    procedure AssignLerp(const A: Double; Value1, Value2: TX3DField); override;
    function CanAssignLerp: boolean; override;
    class function X3DType: String; override;
    class function CreateEvent(const AParentNode: TX3DFileItem; const AName: String; const AInEvent: boolean): TX3DEvent; override;
  end;
{$endif read_interface}
"#;
    let result = extract(source);
    for expected in &["TMFMatrix3f", "TMFMatrix3d"] {
        assert!(
            result.symbols.iter().any(|s| &s.name == expected && s.kind == SymbolKind::Class),
            "expected class {} to be extracted", expected
        );
    }
}

// ---------------------------------------------------------------------------
// Live file: castlefields_x3dsimplemultfield_descendants.inc — class extraction
// ---------------------------------------------------------------------------

#[test]
fn castlefields_simplemult_classes_extracted() {
    use std::fs;
    let path = r"F:\Work\Projects\TestProjects\pascal-castle-fresh\src\scene\x3d\castlefields_x3dsimplemultfield_descendants.inc";
    let src = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return,
    };
    let result = extract(&src);
    let missing: Vec<&str> = ["TMFBool", "TMFLong", "TMFInt32", "TMFVec3f", "TMFFloat", "TMFString", "TMFTime", "TMFColor"]
        .iter()
        .filter(|&&n| !result.symbols.iter().any(|s| s.name == n && s.kind == SymbolKind::Class))
        .copied()
        .collect();
    assert!(missing.is_empty(), "classes missing from simplemultfield_descendants.inc: {:?}", missing);
}

// ---------------------------------------------------------------------------
// Normalisation: FPC generic `{$ifdef FPC}specialize{$endif} Type<A,B,C>` forms
// ---------------------------------------------------------------------------

#[test]
fn normalised_source_strips_specialize_and_generic_params() {
    use super::normalise_source_for_test;
    let source = r#"{$ifdef read_interface}
  TMFMatrix3f = class({$ifdef FPC}specialize{$endif} TX3DSimpleMultField<
    TMatrix3,
    TSFMatrix3f,
    TMatrix3List>)
  strict protected
    function RawItemToString(const ItemNum: Integer): String; override;
  end;

  TMFMatrix3d = class({$ifdef FPC}specialize{$endif} TX3DSimpleMultField<
    TMatrix3Double,
    TSFMatrix3d,
    TMatrix3DoubleList>)
  strict protected
    function RawItemToString(const ItemNum: Integer): String; override;
  end;
{$endif read_interface}
"#;
    let normalised = normalise_source_for_test(source);
    assert!(!normalised.contains('<'), "normalised source still contains '<'");
    assert!(!normalised.contains('>'), "normalised source still contains '>'");
    assert!(!normalised.contains("specialize"), "normalised source still contains 'specialize'");
}
