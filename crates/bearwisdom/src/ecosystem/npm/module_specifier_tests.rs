use super::*;

#[test]
fn npm_candidates_include_definitely_typed_and_package_root_forms() {
    assert_eq!(
        module_prefix_candidates("@scope/pkg/subpath"),
        vec![
            "@scope/pkg/subpath",
            "@types/scope__pkg/subpath",
            "@scope/pkg",
        ]
    );
    assert_eq!(
        module_prefix_candidates("pkg/subpath"),
        vec!["pkg/subpath", "@types/pkg/subpath", "pkg"]
    );
}

#[test]
fn npm_scheme_candidates_are_adapter_owned() {
    assert_eq!(
        module_prefix_candidates("node:assert/strict"),
        vec![
            "node:assert/strict",
            "assert/strict",
            "@types/assert/strict",
            "assert",
            "node/assert/strict",
            "@types/node/assert/strict",
            "node/assert",
            "node",
        ]
    );
}

#[test]
fn npm_directory_fallback_accepts_only_path_specifiers() {
    assert!(declines_directory_match("react"));
    assert!(declines_directory_match("@scope/pkg"));
    assert!(!declines_directory_match("./react"));
    assert!(!declines_directory_match("C:/react"));
}
