use super::*;

#[test]
fn nim_reexport_paths_cover_source_and_mod_entries() {
    assert_eq!(
        relative_reexport_candidate_paths("lib/collections/sets"),
        vec![
            "lib/collections/sets".to_string(),
            "lib/collections/sets.nim".to_string(),
            "lib/collections/sets/mod.nim".to_string(),
        ]
    );
}

#[test]
fn nim_bare_modules_strip_roots_and_match_nim_files() {
    assert!(bare_module_matches_file(
        "lib/pure/strutils.nim",
        "std/strutils"
    ));
    assert!(bare_module_matches_file("vendor/foo/mod.nim", "pkg/foo"));
    assert!(bare_module_matches_file("vendor/foo.nim", "'pkg/foo.nim'"));
    assert!(!bare_module_matches_file("vendor/foo.nim", "std/bar"));
}

#[test]
fn nim_external_import_terms_strip_stdlib_and_package_roots() {
    assert_eq!(
        external_import_match_terms("std/httpclient"),
        vec!["httpclient"]
    );
    assert_eq!(
        external_import_match_terms("pkg/collections/sets"),
        vec!["sets"]
    );
    assert_eq!(
        external_import_match_terms("vendor/collections/sets"),
        vec!["sets", "vendor"]
    );
}
