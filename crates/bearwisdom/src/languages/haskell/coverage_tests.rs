// =============================================================================
// haskell/coverage_tests.rs — Node-kind coverage tests for the Haskell extractor
//
// symbol_node_kinds:
//   function, data_type, newtype, class, instance, type_synomym,
//   foreign_import, foreign_export, pattern_synonym
//
// ref_node_kinds:
//   import, apply, infix
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

/// function → SymbolKind::Function
/// tree-sitter-haskell produces a `function` node when there is a type signature
/// followed by an equation (pattern matching / parameter binding). A bare
/// `foo = 1` produces a `bind` node instead. Use a typed form to guarantee a
/// `function` node.
#[test]
fn cov_function_emits_function_symbol() {
    let src = "foo :: Int -> Int\nfoo x = x + 1\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "foo");
    assert!(sym.is_some(), "expected Function 'foo'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

/// module header → SymbolKind::Namespace  (the module name is extracted)
#[test]
fn cov_module_header_emits_namespace() {
    let src = "module Main where\n";
    let r = extract::extract(src);
    let ns = r.symbols.iter().find(|s| s.kind == SymbolKind::Namespace && s.name == "Main");
    assert!(ns.is_some(), "expected Namespace symbol 'Main' from module header; got: {:?}", r.symbols);
}

/// data_type → SymbolKind::Struct
#[test]
fn cov_data_type_emits_struct() {
    let src = "data Shape = Circle Float | Square Float\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Shape");
    assert!(sym.is_some(), "expected Struct 'Shape'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Struct);
}

/// newtype → SymbolKind::Struct
#[test]
fn cov_newtype_emits_struct() {
    let src = "newtype Name = Name String\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Name");
    assert!(sym.is_some(), "expected Struct 'Name' from newtype; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Struct);
}

/// class → SymbolKind::Interface  (type class)
#[test]
fn cov_class_emits_interface() {
    let src = "class Eq a where\n  eq :: a -> a -> Bool\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Eq");
    assert!(sym.is_some(), "expected Interface 'Eq' from class; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Interface);
}

/// instance → SymbolKind::Class
#[test]
fn cov_instance_emits_class() {
    let src = "instance Eq Bool where\n  eq x y = x == y\n";
    let r = extract::extract(src);
    // instance emits a Class symbol; name includes both class and type names
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Class);
    assert!(sym.is_some(), "expected Class symbol from instance; got: {:?}", r.symbols);
}

/// type_synomym → SymbolKind::TypeAlias
#[test]
fn cov_type_synomym_emits_type_alias() {
    let src = "type Name = String\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Name");
    assert!(sym.is_some(), "expected TypeAlias 'Name' from type_synomym; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::TypeAlias);
}

/// foreign_import → SymbolKind::Function
#[test]
fn cov_foreign_import_emits_function() {
    let src = "foreign import ccall \"strlen\" c_strlen :: Ptr CChar -> IO CSize\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "c_strlen");
    assert!(
        sym.is_some(),
        "expected Function 'c_strlen' from foreign_import; got: {:?}",
        r.symbols
    );
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

/// foreign_export → does not crash; may emit Function
#[test]
fn cov_foreign_export_does_not_crash() {
    let src = "foreign export ccall myFunc :: Int -> IO ()\nmyFunc x = return ()\n";
    let r = extract::extract(src);
    let _ = r;
}

/// pattern_synonym → does not crash (node kind declared but behaviour
/// depends on grammar support for the construct)
#[test]
fn cov_pattern_synonym_does_not_crash() {
    let src = "pattern Zero :: Int\npattern Zero = 0\n";
    let r = extract::extract(src);
    let _ = r;
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

/// import → EdgeKind::Imports
#[test]
fn cov_import_emits_imports_ref() {
    let src = "import Data.List\n";
    let r = extract::extract(src);
    let imports: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Imports)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        imports.contains(&"List"),
        "expected Imports ref to 'List' from import; got: {imports:?}"
    );
}

/// apply → EdgeKind::Calls  (function application)
#[test]
fn cov_apply_emits_calls_ref() {
    let src = "result = map negate [1, 2, 3]\n";
    let r = extract::extract(src);
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        calls.contains(&"map") || calls.contains(&"negate"),
        "expected Calls ref from apply; got: {calls:?}"
    );
}

/// infix → does not crash (infix operator application)
#[test]
fn cov_infix_does_not_crash() {
    let src = "x = 1 `div` 2\n";
    let r = extract::extract(src);
    let _ = r;
}
