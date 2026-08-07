use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::type_checker::profile::language_profile::{ModuleScope, DEFAULT_PROFILE};

fn resolve(
    lookup: &Lookup,
    target: &str,
    src_file: &str,
    profile: &crate::type_checker::profile::language_profile::LanguageProfile,
) -> Option<i64> {
    use crate::indexer::resolve::engine::testkit::call_ref;
    let r = call_ref(target);
    let s = source_symbol("caller");
    // Override the file path in the context so same-dir checks work.
    let mut fc = file_ctx(vec![], None);
    fc.file_path = src_file.to_string();
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile,
    };
    match ModuleScopeRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

/// Gate is `Off` by default — rule always passes.
#[test]
fn passes_when_gate_off() {
    let lookup = Lookup::new().with(sym(1, "Vec2", "Vec2", "struct", "pkg/math/vec.odin"));
    assert_eq!(resolve(&lookup, "Vec2", "pkg/math/main.odin", &DEFAULT_PROFILE), None);
}

/// `SameDir`: binds the first kind-compatible symbol in the same parent dir.
#[test]
fn same_dir_resolves_sibling() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            module_scope: ModuleScope::SameDir,
            ..DEFAULT_PROFILE
        };
    let lookup = Lookup::new().with(sym(10, "Vec2", "Vec2", "struct", "pkg/math/vec.odin"));
    // Source is also in `pkg/math/` — same parent dir.
    assert_eq!(resolve(&lookup, "Vec2", "pkg/math/main.odin", &PROFILE), Some(10));
}

/// `SameDir`: symbol in a different dir is not bound.
#[test]
fn same_dir_declines_different_dir() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            module_scope: ModuleScope::SameDir,
            ..DEFAULT_PROFILE
        };
    let lookup = Lookup::new().with(sym(20, "Vec2", "Vec2", "struct", "pkg/geo/vec.odin"));
    // Source is in `pkg/math/` — different parent dir.
    assert_eq!(resolve(&lookup, "Vec2", "pkg/math/main.odin", &PROFILE), None);
}

/// `SameDirUnique`: binds when exactly one internal candidate survives in dir.
#[test]
fn same_dir_unique_resolves_single_candidate() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            module_scope: ModuleScope::SameDirUnique,
            ..DEFAULT_PROFILE
        };
    let lookup = Lookup::new().with(sym(30, "init", "init", "function", "src/app/a.pas"));
    assert_eq!(resolve(&lookup, "init", "src/app/b.pas", &PROFILE), Some(30));
}

/// `SameDirUnique`: declines when two distinct candidates exist in the same dir.
#[test]
fn same_dir_unique_declines_ambiguous() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            module_scope: ModuleScope::SameDirUnique,
            ..DEFAULT_PROFILE
        };
    let lookup = Lookup::new()
        .with(sym(40, "init", "init", "function", "src/app/a.pas"))
        .with(sym(41, "init", "init2", "function", "src/app/c.pas"));
    assert_eq!(resolve(&lookup, "init", "src/app/b.pas", &PROFILE), None);
}

/// `SourcesTargetSubtree`: binds when exactly one candidate shares the subtree.
#[test]
fn sources_target_subtree_resolves() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            module_scope: ModuleScope::SourcesTargetSubtree,
            ..DEFAULT_PROFILE
        };
    let lookup = Lookup::new().with(sym(
        50,
        "UserService",
        "UserService",
        "class",
        "Sources/App/UserService.swift",
    ));
    assert_eq!(
        resolve(&lookup, "UserService", "Sources/App/Controller.swift", &PROFILE),
        Some(50)
    );
}
