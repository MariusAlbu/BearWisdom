use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{
    DEFAULT_PROFILE, LanguageProfile, ModuleAnchor, ModuleAnchorBind,
};
use crate::types::EdgeKind;

static TERMINAL_ON_PROFILE: LanguageProfile = LanguageProfile {
    module_anchor_terminal: true,
    module_anchor: ModuleAnchor::On(ModuleAnchorBind::NameExactKind),
    ..DEFAULT_PROFILE
};

fn run(
    profile: &LanguageProfile,
    module: Option<&str>,
    edge_kind: EdgeKind,
) -> LookupResult {
    let mut r = call_ref("foo");
    r.module = module.map(|s| s.to_string());
    r.kind = edge_kind;

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
    ModuleAnchorTerminalRule.apply(&ctx)
}

#[test]
fn passes_when_terminal_flag_is_false() {
    // DEFAULT_PROFILE has module_anchor_terminal = false.
    assert!(matches!(
        run(&DEFAULT_PROFILE, Some("mymod"), EdgeKind::Calls),
        LookupResult::Pass
    ));
}

#[test]
fn passes_when_anchor_is_off() {
    static TERMINAL_ANCHOR_OFF: LanguageProfile = LanguageProfile {
        module_anchor_terminal: true,
        module_anchor: ModuleAnchor::Off,
        ..DEFAULT_PROFILE
    };
    assert!(matches!(
        run(&TERMINAL_ANCHOR_OFF, Some("mymod"), EdgeKind::Calls),
        LookupResult::Pass
    ));
}

#[test]
fn passes_when_ref_has_no_module() {
    assert!(matches!(
        run(&TERMINAL_ON_PROFILE, None, EdgeKind::Calls),
        LookupResult::Pass
    ));
}

#[test]
fn passes_for_imports_edge_kind() {
    // The terminal guard exempts Imports edges.
    assert!(matches!(
        run(&TERMINAL_ON_PROFILE, Some("mymod"), EdgeKind::Imports),
        LookupResult::Pass
    ));
}

#[test]
fn stops_when_all_conditions_hold() {
    assert!(matches!(
        run(&TERMINAL_ON_PROFILE, Some("mymod"), EdgeKind::Calls),
        LookupResult::Stop
    ));
}

#[test]
fn default_profile_always_passes() {
    // DEFAULT_PROFILE.module_anchor_terminal = false → always Pass.
    assert!(matches!(
        run(&DEFAULT_PROFILE, Some("mymod"), EdgeKind::Calls),
        LookupResult::Pass
    ));
}
