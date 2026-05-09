use crate::types::SymbolKind;

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

