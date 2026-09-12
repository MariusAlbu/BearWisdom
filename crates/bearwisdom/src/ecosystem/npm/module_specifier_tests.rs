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

#[test]
fn npm_external_paths_own_scoped_package_entry_keys() {
    assert_eq!(
        package_entry_key("ext:ts:@scope/pkg/dist/index.d.ts").as_deref(),
        Some("@scope/pkg")
    );
    assert_eq!(
        package_entry_key("ext:ts:lodash/fp.js").as_deref(),
        Some("lodash")
    );
    assert_eq!(package_entry_key("ext:dart:matcher/expect.dart"), None);
}

#[test]
fn npm_reexport_candidates_keep_js_family_suffixes_in_the_adapter() {
    let candidates = relative_reexport_candidate_paths("packages/q/src/member");
    assert!(candidates.contains(&"packages/q/src/member.ts".to_string()));
    assert!(candidates.contains(&"packages/q/src/member/index.tsx".to_string()));
    assert!(candidates.contains(&"packages/q/src/member.vue".to_string()));
    assert!(candidates.contains(&"packages/q/src/member".to_string()));
}

#[test]
fn a_definitely_typed_path_offers_its_owner_as_an_entry_alias() {
    assert_eq!(entry_aliases("ext:ts:@types/react/index.d.ts"), vec!["react"]);
    assert_eq!(
        entry_aliases("ext:ts:@types/babel__core/index.d.ts"),
        vec!["@babel/core"]
    );
    assert!(entry_aliases("ext:ts:react/index.d.ts").is_empty());
    assert!(entry_aliases("src/app.ts").is_empty());
    assert_eq!(relative_entry_key("src/app.ts", "./x"), None);
}
