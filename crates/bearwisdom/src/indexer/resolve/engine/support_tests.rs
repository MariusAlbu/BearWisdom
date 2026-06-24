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
fn parent_dir_drops_final_segment() {
    assert_eq!(
        parent_dir("schema/users/model.prisma").as_deref(),
        Some("schema/users")
    );
    assert_eq!(parent_dir("bare.rs"), None);
    assert_eq!(parent_dir("a\\b\\c.ts").as_deref(), Some("a/b"));
}

#[test]
fn basename_stem_matches_ignores_directories() {
    assert!(basename_stem_matches("components/synedit/foo.pas", "foo"));
    assert!(!basename_stem_matches("components/synedit/foo.pas", "synedit"));
    assert!(basename_stem_matches("foo", "foo"));
    assert!(!basename_stem_matches("foo.pas", ""));
}

#[test]
fn path_stem_matches_checks_stem_and_segments() {
    assert!(path_stem_matches("a/b/foo.ts", "foo"));
    assert!(path_stem_matches("a/foo/bar.ts", "foo"));
    assert!(path_stem_matches("ext:ruby:aws-sdk-s3/lib/x.rb", "aws-sdk-s3"));
    assert!(!path_stem_matches("a/b/c.ts", "foo"));
}

#[test]
fn trim_source_extension_strips_known_extensions() {
    assert_eq!(trim_source_extension("foo.ts"), "foo");
    assert_eq!(trim_source_extension("foo.vue"), "foo");
    assert_eq!(trim_source_extension("foo.rb"), "foo.rb");
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
fn relative_base_joins_and_collapses_dot_segments() {
    // `./sibling` from a barrel resolves to the barrel's directory.
    assert_eq!(
        relative_base("packages/q/src/index.ts", "./queryClient").as_deref(),
        Some("packages/q/src/queryClient")
    );
    // `..` from a nested test file climbs to the parent directory's stem.
    assert_eq!(
        relative_base("packages/q/src/__tests__/a.test.tsx", "..").as_deref(),
        Some("packages/q/src")
    );
    // No directory portion — no base.
    assert_eq!(relative_base("index.ts", "./x"), None);
}

#[test]
fn relative_file_matches_base_covers_extension_and_index_forms() {
    let base = "packages/q/src/queryClient";
    assert!(relative_file_matches_base(
        "packages/q/src/queryClient.ts",
        base
    ));
    // A deeper indexed path still matches as a suffix.
    assert!(relative_file_matches_base(
        "repo/packages/q/src/queryClient.tsx",
        base
    ));
    // The `/index` directory form of a parent base.
    assert!(relative_file_matches_base(
        "packages/q/src/index.ts",
        "packages/q/src"
    ));
    assert!(!relative_file_matches_base("packages/q/src/other.ts", base));
}

#[test]
fn path_basename_stem_is_index_matches_index_barrels_only() {
    assert!(path_basename_stem_is_index("packages/q/src/index.ts"));
    assert!(path_basename_stem_is_index("a\\b\\index.tsx"));
    assert!(!path_basename_stem_is_index("packages/q/src/queryClient.ts"));
    assert!(!path_basename_stem_is_index("indexer.ts"));
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
