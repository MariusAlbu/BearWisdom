// =============================================================================
// typescript/module_policy_tests — ES-module specifier grammar
// =============================================================================

use super::SOURCE_MODULE_PATH_POLICY as POLICY;
use crate::type_checker::profile::language_profile::ModuleSpecifierClass;

#[test]
fn file_specifiers_are_relative_and_everything_else_is_bare() {
    for spec in ["./utils", "../shared/types", "/abs/path", "C:/work/x", "./comp.vue"] {
        assert_eq!(POLICY.classify(spec), ModuleSpecifierClass::Relative, "{spec}");
    }
    for spec in ["react", "@tanstack/react-query", "node:fs", "@/components", "~/lib", "rxjs/operators"] {
        assert_eq!(POLICY.classify(spec), ModuleSpecifierClass::Bare, "{spec}");
    }
}

#[test]
fn a_relative_base_names_its_source_forms_then_its_directory_entry() {
    let candidates = (POLICY.relative_candidate_paths)("src/ui/badge");
    let position = |path: &str| {
        candidates
            .iter()
            .position(|c| c == path)
            .unwrap_or_else(|| panic!("{path} missing from {candidates:?}"))
    };
    assert_eq!(candidates[0], "src/ui/badge", "the spelled path itself comes first");
    assert!(position("src/ui/badge.ts") < position("src/ui/badge/index.ts"));
    assert!(position("src/ui/badge.tsx") < position("src/ui/badge.vue"));
    for path in ["src/ui/badge.d.ts", "src/ui/badge.svelte", "src/ui/badge/index.vue"] {
        position(path);
    }
}

#[test]
fn an_emitted_extension_names_its_source_before_itself() {
    let candidates = (POLICY.relative_candidate_paths)("src/store.js");
    assert_eq!(
        &candidates[..4],
        ["src/store.ts", "src/store.tsx", "src/store.d.ts", "src/store.js"]
    );
    let candidates = (POLICY.relative_candidate_paths)("src/store.mjs");
    assert_eq!(&candidates[..3], ["src/store.mts", "src/store.d.mts", "src/store.mjs"]);
}

#[test]
fn a_bare_specifier_never_matches_a_project_file_by_spelling() {
    assert!(!(POLICY.bare_module_matches_file)("src/react.ts", "react"));
    assert!((POLICY.external_import_match_terms)("@nestjs/common").is_empty());
}
