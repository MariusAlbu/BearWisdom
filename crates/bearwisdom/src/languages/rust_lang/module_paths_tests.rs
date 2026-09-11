use super::{module_path_match, relative_wildcard_module_files};

#[test]
fn super_glob_candidates_cover_parent_module_file_forms() {
    assert_eq!(
        relative_wildcard_module_files("src/tests/foo_test.rs", "super", "parse"),
        vec!["src/tests/mod.rs", "src/tests.rs"]
    );
}

#[test]
fn crate_glob_candidates_cover_crate_roots() {
    assert_eq!(
        relative_wildcard_module_files("src/foo/bar.rs", "crate", "helper"),
        vec!["src/lib.rs", "src/main.rs"]
    );
}

#[test]
fn non_relative_and_qualified_targets_have_no_candidates() {
    assert!(relative_wildcard_module_files("src/main.rs", "std", "HashMap").is_empty());
    assert!(relative_wildcard_module_files("src/foo.rs", "super", "a::b").is_empty());
}

#[test]
fn crate_directory_hyphen_folding_is_owned_by_the_rust_adapter() {
    let policy = module_path_match("turbo_tasks::vc").source_module_path_policy;
    assert!((policy.bare_module_matches_file)(
        "crates/turbo-tasks/src/vc.rs",
        "turbo_tasks::vc"
    ));
}
