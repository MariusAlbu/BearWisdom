use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, Lookup,
};
use crate::type_checker::profile::language_profile::{HeadAliasBind, DEFAULT_PROFILE};

fn apply(
    lookup: &Lookup,
    target: &str,
    profile: &crate::type_checker::profile::language_profile::LanguageProfile,
) -> Option<i64> {
    let r = call_ref(target);
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile,
    };
    match HeadAliasRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

/// Gate is `Off` by default — rule passes without inspecting the symbol index.
#[test]
fn passes_when_gate_off() {
    let lookup = Lookup::new();
    assert_eq!(apply(&lookup, "aws_vpc.id", &DEFAULT_PROFILE), None);
}

/// Head contains `_` — declined regardless of gate.
#[test]
fn declines_head_with_underscore() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            imports: crate::type_checker::profile::language_profile::ImportAxes {
                head_alias: HeadAliasBind::OnSameFile { require_kind: None },
                ..DEFAULT_PROFILE.imports
            },
            ..DEFAULT_PROFILE
        };
    let lookup = Lookup::new();
    assert_eq!(apply(&lookup, "aws_instance.id", &PROFILE), None);
}

/// Bare target (no dot) — declined because there is no head to strip.
#[test]
fn declines_bare_target() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            imports: crate::type_checker::profile::language_profile::ImportAxes {
                head_alias: HeadAliasBind::OnSameFile { require_kind: None },
                ..DEFAULT_PROFILE.imports
            },
            ..DEFAULT_PROFILE
        };
    let lookup = Lookup::new();
    assert_eq!(apply(&lookup, "Foo", &PROFILE), None);
}

/// Gate on but no in-file symbol matches the head — rule declines.
#[test]
fn declines_when_no_in_file_match() {
    static PROFILE: crate::type_checker::profile::language_profile::LanguageProfile =
        crate::type_checker::profile::language_profile::LanguageProfile {
            implicit_root_types: &[],
            imports: crate::type_checker::profile::language_profile::ImportAxes {
                head_alias: HeadAliasBind::OnSameFile { require_kind: None },
                ..DEFAULT_PROFILE.imports
            },
            ..DEFAULT_PROFILE
        };
    // Testkit Lookup::in_file always returns empty, so the loop body is never
    // entered and the rule must return Pass.
    let lookup = Lookup::new();
    assert_eq!(apply(&lookup, "local.id", &PROFILE), None);
}
