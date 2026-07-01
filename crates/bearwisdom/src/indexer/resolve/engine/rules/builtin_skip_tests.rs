use super::*;
use crate::indexer::resolve::engine::testkit::{accept_any, call_ref, file_ctx, ref_ctx, source_symbol, Lookup};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

static SKIP_ECHO_PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
    crate::type_checker::profile::language_profile::LanguageProfile {
        builtin_skip: Some(|t| t == "echo"),
        ..DEFAULT_PROFILE
    };

fn run_with_target(target: &str, profile: &crate::type_checker::profile::language_profile::LanguageProfile) -> LookupResult {
    let r = call_ref(target);
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
    BuiltinSkipRule.apply(&ctx)
}

#[test]
fn passes_when_builtin_skip_is_none() {
    // DEFAULT_PROFILE has builtin_skip = None; any target passes through.
    assert!(matches!(
        run_with_target("echo", &DEFAULT_PROFILE),
        LookupResult::Pass
    ));
}

#[test]
fn drains_when_skip_returns_true_for_the_target() {
    assert!(matches!(
        run_with_target("echo", &SKIP_ECHO_PROFILE),
        LookupResult::Drained
    ));
}

#[test]
fn passes_when_skip_returns_false_for_the_target() {
    assert!(matches!(
        run_with_target("my_project_function", &SKIP_ECHO_PROFILE),
        LookupResult::Pass
    ));
}
