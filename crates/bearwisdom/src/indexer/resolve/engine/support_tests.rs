use super::*;

#[test]
fn is_type_kind_accepts_class_like_only() {
    assert!(is_type_kind("class"));
    assert!(is_type_kind("interface"));
    assert!(is_type_kind("trait"));
    assert!(!is_type_kind("function"));
    assert!(!is_type_kind("namespace"));
    assert!(!is_type_kind("variable"));
}

#[test]
fn qname_under_module_matches_prefix_and_exact() {
    assert!(qname_under_module("Catalog.Service.List", "Catalog.Service"));
    assert!(qname_under_module("Catalog.Service", "Catalog.Service"));
    assert!(qname_under_module("a.b.c", "a::b"));
    assert!(!qname_under_module("CatalogX.Service", "Catalog"));
    assert!(!qname_under_module("x", ""));
}

#[test]
fn qname_directly_under_requires_one_segment() {
    assert!(qname_directly_under("Assertions.assertTrue", "Assertions"));
    assert!(!qname_directly_under("Assertions.Nested.foo", "Assertions"));
    assert!(!qname_directly_under("Assertions", "Assertions"));
}

#[test]
fn strip_self_keyword_strips_first_matching_prefix() {
    assert_eq!(strip_self_keyword("self.method", &["self"]), "method");
    assert_eq!(strip_self_keyword("this.x", &["self", "this"]), "x");
    assert_eq!(strip_self_keyword("plain", &["self"]), "plain");
    assert_eq!(strip_self_keyword("selfish", &["self"]), "selfish");
    assert_eq!(strip_self_keyword("name", &[]), "name");
}

#[test]
fn normalize_name_is_identity_for_none() {
    let out = normalize_name(NameNormalization::None, "FooBar");
    assert_eq!(out, "FooBar");
    assert!(matches!(out, std::borrow::Cow::Borrowed(_)));
}

#[test]
fn path_proximity_score_shared_dir() {
    assert_eq!(path_proximity_score("src/views/Page.ts", "src/views/Helper.ts"), 20);
}

#[test]
fn path_proximity_score_no_overlap() {
    assert_eq!(path_proximity_score("src/a/X.ts", "lib/b/Y.ts"), 0);
}

#[test]
fn path_proximity_score_partial_overlap() {
    assert_eq!(path_proximity_score("src/a/b/X.ts", "src/a/c/Y.ts"), 20);
}

#[test]
fn implicit_wildcard_namespace_scopes_ranked_pick() {
    use crate::indexer::resolve::engine::testkit::{sym, Lookup};

    // Two external classes share the bare name; only one sits under a
    // manifest-declared implicit namespace (`<Using Include="Xunit"/>`).
    // The scoped one must win the ranked pick even though the decoy has
    // the lower id (the insertion-order/first-winner fallback).
    let decoy = sym(
        1,
        "Assert",
        "NetTopologySuite.Utilities.Assert",
        "class",
        "ext:dotnet-type:/nts.dll!!nts!!NetTopologySuite.Utilities.Assert",
    );
    let scoped = sym(
        2,
        "Assert",
        "Xunit.Assert",
        "class",
        "ext:dotnet-type:/xa.dll!!xa!!Xunit.Assert",
    );
    let lookup = Lookup::new()
        .with(decoy.clone())
        .with(scoped.clone())
        .with_implicit_namespaces(&["Xunit"]);
    let fc = FileContext {
        file_path: "tests/ParserTests.cs".into(),
        language: "csharp".into(),
        imports: Vec::new(),
        file_namespace: None,
    };
    let cands = [&decoy, &scoped];
    let picked = pick_ranked_candidate(&fc, None, &lookup, &cands)
        .expect("scoped candidate must win by more than the rank margin");
    assert_eq!(picked.qualified_name, "Xunit.Assert");
}
