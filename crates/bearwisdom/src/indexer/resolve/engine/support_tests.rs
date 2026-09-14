use super::*;
use crate::type_checker::profile::language_profile::{
    LanguageProfile, ReceiverSpelling, DEFAULT_PROFILE,
};

static COLON_COLON_PROFILE: LanguageProfile = LanguageProfile {
    qname_separator: "::",
    ..DEFAULT_PROFILE
};

static RUST_WORKSPACE_PROFILE: LanguageProfile = LanguageProfile {
    qname_separator: "::",
    imports: crate::type_checker::profile::language_profile::ImportAxes {
        self_package_root: Some("crate"),
        ..DEFAULT_PROFILE.imports
    },
    ..DEFAULT_PROFILE
};

#[test]
fn workspace_paths_are_profile_normalized_before_neutral_peeling() {
    use crate::indexer::resolve::engine::testkit::Lookup;

    let lookup = Lookup::new()
        .with_workspace_pkg("tantivy", 1)
        .with_workspace_pkg("@scope/pkg", 2);

    assert_eq!(
        workspace_sub_path(&RUST_WORKSPACE_PROFILE, "tantivy::schema", &lookup),
        Some("schema".to_string())
    );
    assert_eq!(
        workspace_sub_path(&DEFAULT_PROFILE, "@scope/pkg/sub", &lookup),
        Some("sub".to_string())
    );
    assert_eq!(
        self_package_sub_path(&RUST_WORKSPACE_PROFILE, "crate::schema"),
        Some(Some("schema".to_string()))
    );
}

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
