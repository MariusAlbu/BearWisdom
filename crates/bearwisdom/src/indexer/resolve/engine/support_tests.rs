use super::*;
use crate::type_checker::profile::language_profile::{
    LanguageProfile, ReceiverSpelling, DEFAULT_PROFILE,
};

static COLON_COLON_PROFILE: LanguageProfile = LanguageProfile {
    qname_separator: "::",
    ..DEFAULT_PROFILE
};

#[test]
fn qname_under_module_matches_prefix_and_exact() {
    assert!(qname_under_module(
        &DEFAULT_PROFILE,
        "Catalog.Service.List",
        "Catalog.Service"
    ));
    assert!(qname_under_module(
        &DEFAULT_PROFILE,
        "Catalog.Service",
        "Catalog.Service"
    ));
    assert!(qname_under_module(&COLON_COLON_PROFILE, "a.b.c", "a::b"));
    assert!(!qname_under_module(&DEFAULT_PROFILE, "a.b.c", "a::b"));
    assert!(!qname_under_module(
        &DEFAULT_PROFILE,
        "CatalogX.Service",
        "Catalog"
    ));
    assert!(!qname_under_module(&DEFAULT_PROFILE, "x", ""));
}

#[test]
fn qname_directly_under_requires_one_segment() {
    assert!(qname_directly_under(
        &DEFAULT_PROFILE,
        "Assertions.assertTrue",
        "Assertions"
    ));
    assert!(!qname_directly_under(
        &DEFAULT_PROFILE,
        "Assertions.Nested.foo",
        "Assertions"
    ));
    assert!(!qname_directly_under(
        &DEFAULT_PROFILE,
        "Assertions",
        "Assertions"
    ));
}

#[test]
fn receiver_member_normalization_uses_profile_declared_separator() {
    static PROFILE: LanguageProfile = LanguageProfile {
        receiver_spellings: &[
            ReceiverSpelling::enclosing("self", "."),
            ReceiverSpelling::enclosing("Self", "::"),
        ],
        ..DEFAULT_PROFILE
    };
    assert_eq!(
        PROFILE.normalize_receiver_member_target("self.method"),
        "method"
    );
    assert_eq!(
        PROFILE.normalize_receiver_member_target("Self::method"),
        "method"
    );
    assert_eq!(
        PROFILE.normalize_receiver_member_target("Self.method"),
        "Self.method"
    );
    assert_eq!(
        PROFILE.normalize_receiver_member_target("selfish"),
        "selfish"
    );
    assert_eq!(
        DEFAULT_PROFILE.normalize_receiver_member_target("name"),
        "name"
    );
}

#[test]
fn normalize_name_is_identity_for_none() {
    let out = normalize_name(NameNormalization::None, "FooBar");
    assert_eq!(out, "FooBar");
    assert!(matches!(out, std::borrow::Cow::Borrowed(_)));
}

#[test]
fn path_proximity_score_shared_dir() {
    assert_eq!(
        path_proximity_score("src/views/Page.ts", "src/views/Helper.ts"),
        20
    );
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
