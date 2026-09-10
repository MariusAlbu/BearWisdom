use super::*;

fn test_relative_candidate_paths(base: &str) -> Vec<String> {
    vec![
        base.to_string(),
        format!("{base}.source"),
        format!("{base}/entry.source"),
    ]
}

fn never_matches_bare_module(_file_path: &str, _source_module: &str) -> bool {
    false
}

const TEST_PATH_POLICY: SourceModulePathPolicy = SourceModulePathPolicy {
    classify_specifier: |_| {
        crate::type_checker::profile::language_profile::ModuleSpecifierClass::Relative
    },
    relative_candidate_paths: test_relative_candidate_paths,
    bare_module_matches_file: never_matches_bare_module,
    external_import_match_terms: |_| Vec::new(),
};

#[test]
fn relative_base_joins_and_collapses_dot_segments() {
    assert_eq!(
        relative_base("packages/q/src/barrel.source", "./member").as_deref(),
        Some("packages/q/src/member")
    );
    assert_eq!(
        relative_base("packages/q/src/nested/barrel.source", "..").as_deref(),
        Some("packages/q/src")
    );
    assert_eq!(relative_base("barrel.source", "./member"), None);
}

#[test]
fn relative_file_matches_profile_supplied_candidates() {
    let base = "packages/q/src/member";
    assert!(relative_file_matches_base(
        "packages/q/src/member.source",
        base,
        TEST_PATH_POLICY,
    ));
    assert!(relative_file_matches_base(
        "repo/packages/q/src/member/entry.source",
        base,
        TEST_PATH_POLICY,
    ));
    assert!(!relative_file_matches_base(
        "packages/q/src/other.source",
        base,
        TEST_PATH_POLICY,
    ));
}

#[test]
fn unsupported_policy_has_no_path_candidates() {
    let policy = SourceModulePathPolicy::unsupported();
    assert!(!relative_file_matches_base(
        "lib/member.unit",
        "lib/member.unit",
        policy,
    ));
}
