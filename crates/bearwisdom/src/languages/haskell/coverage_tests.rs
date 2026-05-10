// =============================================================================
// haskell/coverage_tests.rs — Node-kind coverage tests for the Haskell extractor
//
// symbol_node_kinds:
//   function, data_type, newtype, class, instance, type_synomym,
//   foreign_import, foreign_export, pattern_synonym,
//   data_family, type_family, data_constructor (TODO), gadt_constructor (TODO)
//
// ref_node_kinds:
//   import, apply, infix, instance→Implements, deriving→Implements,
//   class method→Method, instance method→Method
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

/// Qualified call: Map.lookup → target_name = "lookup", module = Some("Map")
#[test]
fn ref_qualified_call() {
    let src = "module M where\nimport qualified Data.Map as Map\nf x = Map.lookup x Map.empty\n";
    let r = extract::extract(src);
    let rf = r.refs.iter().find(|rf| rf.target_name == "lookup" && rf.kind == EdgeKind::Calls);
    assert!(
        rf.is_some(),
        "expected Calls ref to 'lookup'; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert_eq!(
        rf.unwrap().module.as_deref(),
        Some("Map"),
        "expected module=Some(\"Map\") for qualified call Map.lookup"
    );
}

/// Nested qualified call: Data.Map.lookup → module = "Data.Map", target = "lookup"
#[test]
fn ref_nested_qualified_call() {
    let src = "module M where\nf x = Data.Map.lookup x Data.Map.empty\n";
    let r = extract::extract(src);
    let rf = r.refs.iter().find(|rf| rf.target_name == "lookup" && rf.kind == EdgeKind::Calls);
    assert!(
        rf.is_some(),
        "expected Calls ref to 'lookup'; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert_eq!(
        rf.unwrap().module.as_deref(),
        Some("Data.Map"),
        "expected module=Some(\"Data.Map\") for nested qualified call Data.Map.lookup"
    );
}

// ---------------------------------------------------------------------------
// Additional symbol_node_kinds
// ---------------------------------------------------------------------------

/// data_family → SymbolKind::Struct
#[test]
fn cov_data_family_emits_struct() {
    let src = "data family XMap v\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "XMap");
    assert!(sym.is_some(), "expected Struct 'XMap' from data_family; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Struct);
}

/// type_family → SymbolKind::TypeAlias
#[test]
fn cov_type_family_emits_type_alias() {
    let src = "type family Elem c\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "Elem");
    assert!(sym.is_some(), "expected TypeAlias 'Elem' from type_family; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::TypeAlias);
}

/// Record field names → SymbolKind::Function
/// `data T = T { fieldA :: Int, fieldB :: Bool }` must emit Function symbols
/// for `fieldA` and `fieldB`. Haskell generates accessor functions for every
/// record field that is callable anywhere the type is imported.
#[test]
fn cov_record_field_names_emit_functions() {
    let src = "data SResponse = SResponse\n    { simpleStatus :: Int\n    , simpleHeaders :: [String]\n    , simpleBody :: String\n    }\n";
    let r = extract::extract(src);
    let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
    for field in ["simpleStatus", "simpleHeaders", "simpleBody"] {
        assert!(
            names.iter().any(|&n| n == field),
            "expected record field accessor '{field}' as Function; symbols: {names:?}"
        );
    }
    let field_sym = r.symbols.iter().find(|s| s.name == "simpleStatus");
    assert_eq!(field_sym.map(|s| s.kind), Some(SymbolKind::Function));
}

/// data_constructor → SymbolKind::EnumMember
#[test]
fn cov_data_constructor_emits_enum_member() {
    let src = "data Shape = Circle Float | Square Float\n";
    let r = extract::extract(src);
    // data_type itself
    let sym = r.symbols.iter().find(|s| s.name == "Shape");
    assert!(sym.is_some(), "expected Struct 'Shape'; got: {:?}", r.symbols);
    // constructors
    assert!(
        r.symbols.iter().any(|s| s.name == "Circle" && s.kind == SymbolKind::EnumMember),
        "expected EnumMember 'Circle'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Square" && s.kind == SymbolKind::EnumMember),
        "expected EnumMember 'Square'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// gadt_constructor → SymbolKind::EnumMember
#[test]
fn cov_gadt_constructor_emits_enum_member() {
    let src = concat!(
        "{-# LANGUAGE GADTs #-}\n",
        "data Expr a where\n",
        "  Lit :: Int -> Expr Int\n",
        "  Add :: Expr Int -> Expr Int -> Expr Int\n",
    );
    let r = extract::extract(src);
    // Verify no panic and constructors are extracted
    assert!(
        r.symbols.iter().any(|s| s.name == "Lit" && s.kind == SymbolKind::EnumMember)
        || r.symbols.iter().any(|s| s.name == "Add" && s.kind == SymbolKind::EnumMember),
        "expected at least one gadt_constructor EnumMember (Lit or Add); got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Additional ref_node_kinds
// ---------------------------------------------------------------------------

/// instance → EdgeKind::Implements
/// `instance Show Bool` should emit an Implements edge from the instance
/// symbol to the type class "Show".
#[test]
fn ref_instance_emits_implements() {
    let src = "instance Show Bool where\n  show True = \"True\"\n  show False = \"False\"\n";
    let r = extract::extract(src);
    let imp = r.refs.iter().find(|rf| {
        rf.kind == EdgeKind::Implements && rf.target_name == "Show"
    });
    assert!(
        imp.is_some(),
        "expected Implements ref to 'Show' from instance; got: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// deriving → EdgeKind::Implements
#[test]
fn ref_deriving_emits_implements() {
    let src = "data Foo = Foo deriving (Show, Eq)\n";
    let r = extract::extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Implements && rf.target_name.contains("Show"))
        || r.refs.iter().any(|rf| rf.kind == EdgeKind::Implements && rf.target_name.contains("Eq")),
        "expected Implements ref to 'Show' or 'Eq' from deriving; got: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// class body function → SymbolKind::Method
/// Functions declared inside a `class` body should emit Method, not Function.
#[test]
fn cov_class_method_emits_method() {
    let src = "class Container f where\n  empty :: f a\n  insert :: a -> f a -> f a\n";
    let r = extract::extract(src);
    let method = r.symbols.iter().find(|s| s.kind == SymbolKind::Method);
    assert!(
        method.is_some(),
        "expected at least one Method symbol from class body; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// instance body function → SymbolKind::Method
/// Functions declared inside an `instance` body should emit Method.
#[test]
fn cov_instance_method_emits_method() {
    let src = "instance Show Int where\n  show n = \"<int>\"\n";
    let r = extract::extract(src);
    let method = r.symbols.iter().find(|s| s.kind == SymbolKind::Method);
    assert!(
        method.is_some(),
        "expected at least one Method symbol from instance body; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// infix → EdgeKind::Calls  (backtick infix emits a Calls ref with the operator name)
#[test]
fn ref_infix_emits_calls() {
    let src = "foo :: Int -> Int -> Bool\nfoo x y = x `elem` [y]\n";
    let r = extract::extract(src);
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::Calls)
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(
        calls.contains(&"elem"),
        "expected Calls ref to 'elem' from infix; got: {:?}",
        calls
    );
}

#[test]
#[ignore]
fn debug_probe_haskell_data_type_grammar() {
    let src = "data Shape = Circle Float | Square Float\n";
    let lang: tree_sitter::Language = tree_sitter_haskell::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();
    let tree = parser.parse(src, None).unwrap();
    fn pt(node: tree_sitter::Node, src: &str, depth: usize) {
        let indent = "  ".repeat(depth);
        let text = if node.child_count() == 0 {
            format!(" = {:?}", &src[node.start_byte()..node.end_byte()])
        } else { String::new() };
        eprintln!("{}[{}]{}", indent, node.kind(), text);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) { pt(child, src, depth + 1); }
    }
    pt(tree.root_node(), src, 0);
}

#[test]
#[ignore]
fn debug_probe_haskell_deriving_grammar() {
    let src = "data Foo = Foo deriving (Show, Eq)\n";
    let lang: tree_sitter::Language = tree_sitter_haskell::LANGUAGE.into();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();
    let tree = parser.parse(src, None).unwrap();
    fn pt(node: tree_sitter::Node, src: &str, depth: usize) {
        let indent = "  ".repeat(depth);
        let text = if node.child_count() == 0 {
            format!(" = {:?}", &src[node.start_byte()..node.end_byte()])
        } else { String::new() };
        eprintln!("{}[{}]{}", indent, node.kind(), text);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) { pt(child, src, depth + 1); }
    }
    pt(tree.root_node(), src, 0);
}
