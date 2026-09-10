use super::*;
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

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
    assert!(!basename_stem_matches(
        "components/synedit/foo.pas",
        "synedit"
    ));
    assert!(basename_stem_matches("foo", "foo"));
    assert!(!basename_stem_matches("foo.pas", ""));
}

#[test]
fn path_stem_matches_checks_stem_and_segments() {
    assert!(path_stem_matches("a/b/foo.ts", "foo"));
    assert!(path_stem_matches("a/foo/bar.ts", "foo"));
    assert!(path_stem_matches(
        "ext:ruby:aws-sdk-s3/lib/x.rb",
        "aws-sdk-s3"
    ));
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
fn file_path_matches_module_covers_the_three_forms() {
    // Stem suffix — relative import.
    assert!(file_path_matches_module(
        "src/app/foo.ts",
        "./foo",
        &DEFAULT_PROFILE
    ));
    // Dot-to-slash suffix — dotted FQN whose leaf names the file.
    assert!(file_path_matches_module(
        "ext:idx:C:/cache/jdk-src/java.base/java/util/Map.java",
        "java.util.Map",
        &DEFAULT_PROFILE
    ));
    // Segment-bounded run — package directory inside a deeper path.
    assert!(file_path_matches_module(
        "site-packages/posthog/models/__init__.py",
        "posthog.models",
        &DEFAULT_PROFILE
    ));
    // Boundary check: a segment run must not match inside a longer segment.
    assert!(!file_path_matches_module(
        "src/posthog_models/x.py",
        "posthog.models",
        &DEFAULT_PROFILE
    ));
    assert!(!file_path_matches_module(
        "src/a/b.ts",
        "",
        &DEFAULT_PROFILE
    ));
}

#[test]
fn qualified_module_path_variants_are_produced_by_the_active_profile() {
    let mut profile = DEFAULT_PROFILE;
    profile.qname_separator = "::";
    assert!(file_path_matches_module(
        "vendor/pkg/api/Client.rs",
        "pkg::api::Client",
        &profile
    ));
    // A colon-qualified profile does not silently treat a dotted source
    // spelling as a package path.
    assert!(!file_path_matches_module(
        "vendor/pkg/api/Client.rs",
        "pkg.api.Client",
        &profile
    ));
}

#[test]
fn module_specifier_kind_is_adapter_owned() {
    let js = crate::ecosystem::npm::module_specifier::SOURCE_MODULE_PATH_POLICY;
    assert!(js.is_bare("lodash"));
    assert!(!js.is_bare("./local"));
    assert!(js.is_relative("./local"));
    assert!(js.is_relative("C:/drive"));

    // A profile with no adapter cannot accidentally inherit JavaScript's
    // slash and drive grammar.
    let unsupported =
        crate::type_checker::profile::language_profile::SourceModulePathPolicy::unsupported();
    assert!(!unsupported.is_bare("lodash"));
    assert!(!unsupported.is_relative("./local"));
}
