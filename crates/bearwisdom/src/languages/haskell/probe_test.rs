use crate::types::{EdgeKind, SymbolKind};

#[test]
fn import_alias_captured_in_target_name() {
    // `import qualified Data.Text as T` must produce an Imports ref whose
    // target_name is "T" (the alias) and module is "Data.Text". The resolver
    // uses target_name as the alias key so that calls like `T.isPrefixOf`
    // (stored with module="T") are mapped to the correct module.
    let src = "module M where\nimport qualified Data.Text as T\n";
    let r = crate::languages::haskell::extract::extract(src);
    let imp = r.refs.iter().find(|rf| rf.kind == EdgeKind::Imports);
    assert!(imp.is_some(), "expected an Imports ref; got: {:?}", r.refs);
    let imp = imp.unwrap();
    assert_eq!(imp.target_name, "T", "alias should be target_name; got {:?}", imp.target_name);
    assert_eq!(
        imp.module.as_deref(), Some("Data.Text"),
        "module should be the full module name; got {:?}", imp.module
    );
}

#[test]
fn import_without_alias_uses_last_component() {
    // Plain `import Data.Map` should produce target_name="Map", module="Data.Map".
    let src = "module M where\nimport Data.Map\n";
    let r = crate::languages::haskell::extract::extract(src);
    let imp = r.refs.iter().find(|rf| rf.kind == EdgeKind::Imports);
    assert!(imp.is_some(), "expected an Imports ref");
    let imp = imp.unwrap();
    assert_eq!(imp.target_name, "Map");
    assert_eq!(imp.module.as_deref(), Some("Data.Map"));
}

#[test]
fn dotted_variable_split_into_module_and_name() {
    // `T.isPrefixOf x y` — when tree-sitter parses `T.isPrefixOf` as a
    // `variable` node (not a `qualified` node), the extractor must split at
    // the last `.` and produce target_name="isPrefixOf", module=Some("T").
    let src = "module M where\nf x y = T.isPrefixOf x y\n";
    let r = crate::languages::haskell::extract::extract(src);
    let call = r.refs.iter().find(|rf| {
        rf.kind == EdgeKind::Calls && rf.target_name == "isPrefixOf"
    });
    assert!(
        call.is_some(),
        "expected Calls ref to 'isPrefixOf'; got: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind, &rf.module)).collect::<Vec<_>>()
    );
    let call = call.unwrap();
    assert_eq!(call.module.as_deref(), Some("T"), "expected module=Some(\"T\"); got {:?}", call.module);
}

#[test]
fn class_with_multi_name_operator_signature_emits_methods() {
    let src = r#"
class Eq a where
    (==), (/=) :: a -> a -> Bool

class Num a where
    (+), (-), (*) :: a -> a -> a
"#;
    let r = crate::languages::haskell::extract::extract(src);
    let names: Vec<(&str, SymbolKind)> =
        r.symbols.iter().map(|s| (s.name.as_str(), s.kind)).collect();
    for op in ["==", "/=", "+", "-", "*"] {
        assert!(
            names.iter().any(|(n, k)| *n == op && matches!(k, SymbolKind::Method)),
            "expected {op} as Method symbol from class declaration; got: {names:?}"
        );
    }
}

#[test]
fn data_with_operator_constructor_emits_cons() {
    // `a : List a` — Haskell's list cons constructor is an operator
    // data-constructor. The Haskell extractor must surface `:` as an
    // EnumMember symbol so refs to `(x : xs)` patterns can resolve.
    let src = "data List a = [] | a : List a\n";
    let r = crate::languages::haskell::extract::extract(src);
    let names: Vec<&str> = r.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.iter().any(|n| *n == ":"),
        "expected `:` constructor from `data List a = ... | a : List a`; got {names:?}"
    );
    assert!(
        names.iter().any(|n| *n == "[]"),
        "expected `[]` nullary constructor; got {names:?}"
    );
}

