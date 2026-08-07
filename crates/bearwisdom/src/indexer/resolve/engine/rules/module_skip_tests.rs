use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

static SKIP_STD_PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
    crate::type_checker::profile::language_profile::LanguageProfile {
        implicit_root_types: &[],
        module_skip: Some(|m| m == "std"),
        ..DEFAULT_PROFILE
    };

fn run_with_module(module: Option<&str>, profile: &crate::type_checker::profile::language_profile::LanguageProfile) -> LookupResult {
    let mut r = call_ref("foo");
    r.module = module.map(|s| s.to_string());

    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let lookup = Lookup::new();

    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile,
    };
    ModuleSkipRule.apply(&ctx)
}

#[test]
fn passes_when_module_skip_is_none() {
    // DEFAULT_PROFILE has module_skip = None; any module on the ref passes through.
    assert!(matches!(
        run_with_module(Some("std"), &DEFAULT_PROFILE),
        LookupResult::Pass
    ));
}

#[test]
fn passes_when_ref_has_no_module() {
    assert!(matches!(
        run_with_module(None, &SKIP_STD_PROFILE),
        LookupResult::Pass
    ));
}

#[test]
fn stops_when_skip_returns_true_for_the_module() {
    assert!(matches!(
        run_with_module(Some("std"), &SKIP_STD_PROFILE),
        LookupResult::Stop
    ));
}

#[test]
fn passes_when_skip_returns_false_for_the_module() {
    assert!(matches!(
        run_with_module(Some("mylib"), &SKIP_STD_PROFILE),
        LookupResult::Pass
    ));
}

#[test]
fn default_profile_always_passes() {
    assert!(matches!(
        run_with_module(Some("std"), &DEFAULT_PROFILE),
        LookupResult::Pass
    ));
}
