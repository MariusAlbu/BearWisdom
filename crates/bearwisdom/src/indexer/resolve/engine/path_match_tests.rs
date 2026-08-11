use super::*;

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
fn trim_path_extension_strips_any_basename_extension() {
    assert_eq!(trim_path_extension("a/b/Map.java"), "a/b/Map");
    assert_eq!(trim_path_extension("a/b/foo.spec.ts"), "a/b/foo.spec");
    assert_eq!(trim_path_extension("a.b/binary"), "a.b/binary");
    assert_eq!(trim_path_extension("Makefile"), "Makefile");
    assert_eq!(trim_path_extension("foo.rs"), "foo");
}

#[test]
fn trim_source_extension_strips_known_extensions() {
    assert_eq!(trim_source_extension("foo.ts"), "foo");
    assert_eq!(trim_source_extension("foo.vue"), "foo");
    assert_eq!(trim_source_extension("foo.rb"), "foo.rb");
}

#[test]
fn file_path_matches_module_covers_the_three_forms() {
    // Stem suffix — relative import.
    assert!(file_path_matches_module("src/app/foo.ts", "./foo"));
    // Dot-to-slash suffix — dotted FQN whose leaf names the file.
    assert!(file_path_matches_module(
        "ext:idx:C:/cache/jdk-src/java.base/java/util/Map.java",
        "java.util.Map"
    ));
    // Segment-bounded run — package directory inside a deeper path.
    assert!(file_path_matches_module(
        "site-packages/posthog/models/__init__.py",
        "posthog.models"
    ));
    // Boundary check: a segment run must not match inside a longer segment.
    assert!(!file_path_matches_module("src/posthog_models/x.py", "posthog.models"));
    assert!(!file_path_matches_module("src/a/b.ts", ""));
}

#[test]
fn specifier_kind_predicates_split_on_leading_shape() {
    assert!(is_bare_module_specifier("lodash"));
    assert!(!is_bare_module_specifier("./local"));
    assert!(!is_bare_module_specifier("/abs/path"));
    assert!(!is_bare_module_specifier("C:/drive"));
    assert!(is_relative_specifier("./local"));
    assert!(is_relative_specifier("C:/drive"));
    assert!(!is_relative_specifier("lodash"));
}
