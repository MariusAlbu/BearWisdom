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
    assert_eq!(
        imp.target_name, "T",
        "alias should be target_name; got {:?}",
        imp.target_name
    );
    assert_eq!(
        imp.module.as_deref(),
        Some("Data.Text"),
        "module should be the full module name; got {:?}",
        imp.module
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
    let call = r
        .refs
        .iter()
        .find(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "isPrefixOf");
    assert!(
        call.is_some(),
        "expected Calls ref to 'isPrefixOf'; got: {:?}",
        r.refs
            .iter()
            .map(|rf| (&rf.target_name, rf.kind, &rf.module))
            .collect::<Vec<_>>()
    );
    let call = call.unwrap();
    assert_eq!(
        call.module.as_deref(),
        Some("T"),
        "expected module=Some(\"T\"); got {:?}",
        call.module
    );
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
    let names: Vec<(&str, SymbolKind)> = r
        .symbols
        .iter()
        .map(|s| (s.name.as_str(), s.kind))
        .collect();
    for op in ["==", "/=", "+", "-", "*"] {
        assert!(
            names
                .iter()
                .any(|(n, k)| *n == op && matches!(k, SymbolKind::Method)),
            "expected {op} as Method symbol from class declaration; got: {names:?}"
        );
    }
}

/// The tyvars an extractor records for a symbol surface as a leading
/// `<...>` clause on the symbol signature — the same channel the index
/// build's generic-param scan already reads for every `<>`/`[]` language.
/// Parse that clause back out so the extractor tests can assert on the
/// recorded tyvar set directly.
fn sig_generics(sig: Option<&str>) -> Vec<String> {
    let sig = sig.unwrap_or("");
    let Some(start) = sig.find('<') else {
        return Vec::new();
    };
    let Some(end_rel) = sig[start..].find('>') else {
        return Vec::new();
    };
    sig[start + 1..start + end_rel]
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[test]
fn constraint_tyvar_recorded_as_generic_param() {
    // `f :: Ord a => a -> a -> Bool` — the constraint context `Ord a`
    // introduces type variable `a` (lowercase, the tyvar) under class `Ord`
    // (uppercase, NOT a tyvar). The extractor must record `a` as a generic
    // param of `f` (carried on the signature's leading `<...>` clause) so the
    // resolver's generic-param strategy can bind constraint occurrences.
    let src = "f :: Ord a => a -> a -> Bool\nf x y = x == y\n";
    let r = crate::languages::haskell::extract::extract(src);
    let f = r
        .symbols
        .iter()
        .find(|s| s.name == "f" && s.signature.is_some())
        .expect("expected signature symbol `f`");
    let gens = sig_generics(f.signature.as_deref());
    assert!(
        gens.iter().any(|p| p == "a"),
        "expected `a` among f generics; got {:?} (sig {:?})",
        gens,
        f.signature
    );
}

#[test]
fn multi_constraint_tuple_collects_all_tyvars() {
    // `g :: (Ord a, Show b) => a -> b -> String` — the constraint is a tuple
    // of two single-param constraints. Collect both tyvars `a` and `b`.
    let src = "g :: (Ord a, Show b) => a -> b -> String\n";
    let r = crate::languages::haskell::extract::extract(src);
    let g = r
        .symbols
        .iter()
        .find(|s| s.name == "g" && s.signature.is_some())
        .expect("expected signature symbol `g`");
    let gens = sig_generics(g.signature.as_deref());
    assert!(
        gens.iter().any(|p| p == "a") && gens.iter().any(|p| p == "b"),
        "expected {{a,b}} ⊆ g generics; got {:?}",
        gens
    );
}

#[test]
fn constraint_class_name_not_a_tyvar() {
    // Soundness hinge: the class name `Ord` (uppercase) must NEVER be recorded
    // as a generic param — only the lowercase tyvar is. An uppercase name in
    // generic_params would let the generic-param strategy falsely bind a
    // type-constructor reference to the wrong declaring symbol.
    let src = "f :: Ord a => a -> a -> Bool\nf x y = x == y\n";
    let r = crate::languages::haskell::extract::extract(src);
    let f = r
        .symbols
        .iter()
        .find(|s| s.name == "f" && s.signature.is_some())
        .unwrap();
    let gens = sig_generics(f.signature.as_deref());
    assert!(
        !gens.iter().any(|p| p == "Ord"),
        "class name `Ord` must not be a generic param; got {:?}",
        gens
    );
}

#[test]
fn forall_quantified_tyvars_collected() {
    // `forall a b. (Eq a) => a -> b -> Bool` — the explicit `forall`
    // quantifier names both tyvars; collect `a` and `b` (superset of the
    // constraint tyvars).
    let src = "h :: forall a b. (Eq a) => a -> b -> Bool\n";
    let r = crate::languages::haskell::extract::extract(src);
    let h = r
        .symbols
        .iter()
        .find(|s| s.name == "h" && s.signature.is_some())
        .unwrap();
    let gens = sig_generics(h.signature.as_deref());
    assert!(
        gens.iter().any(|p| p == "a") && gens.iter().any(|p| p == "b"),
        "expected {{a,b}} ⊆ h generics; got {:?}",
        gens
    );
}

#[test]
fn constraint_tyvar_emits_observable_ref() {
    // The tyvar's occurrence inside the constraint clause is emitted as a
    // TypeRef ref sourced from the declaring function, so the bind is
    // observable to the resolver (and lockable by the resolve test).
    let src = "f :: Ord a => a -> a -> Bool\nf x y = x == y\n";
    let r = crate::languages::haskell::extract::extract(src);
    let f_idx = r
        .symbols
        .iter()
        .position(|s| s.name == "f")
        .expect("symbol `f`");
    assert!(
        r.refs.iter().any(|rf| {
            rf.kind == crate::types::EdgeKind::TypeRef
                && rf.target_name == "a"
                && rf.source_symbol_index == f_idx
        }),
        "expected a TypeRef to `a` sourced from `f`; got {:?}",
        r.refs
            .iter()
            .map(|rf| (&rf.target_name, rf.kind))
            .collect::<Vec<_>>()
    );
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
