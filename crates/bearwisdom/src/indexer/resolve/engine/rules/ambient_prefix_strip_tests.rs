use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

#[test]
fn passes_when_no_prefixes_configured() {
    // DEFAULT_PROFILE has ambient_namespace_prefixes = &[], so always Pass.
    let lookup =
        Lookup::new().with_ambient(sym(1, "concat", "concat", "function", "ext:ts:lib/util.d.ts"));
    let r = call_ref("sys.concat");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    assert!(matches!(AmbientPrefixStripRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn strips_prefix_and_resolves_ambient_leaf() {
    let lookup = Lookup::new().with_ambient(sym(
        10,
        "concat",
        "concat",
        "function",
        "node_modules/@types/utils/index.d.ts",
    ));
    let r = call_ref("sys.concat");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let mut p = DEFAULT_PROFILE;
    p.ambient_namespace_prefixes = &["sys"];
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &p,
    };
    match AmbientPrefixStripRule.apply(&ctx) {
        LookupResult::Resolved(res) => {
            assert_eq!(res.target_symbol_id, 10);
            assert_eq!(res.strategy, "ambient_prefix_strip");
        }
        _ => panic!("expected Resolved"),
    }
}

#[test]
fn passes_when_prefix_not_present_in_target() {
    let lookup = Lookup::new().with_ambient(sym(
        20,
        "resourceId",
        "resourceId",
        "function",
        "node_modules/@types/azure/index.d.ts",
    ));
    let r = call_ref("resourceId"); // no prefix
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let mut p = DEFAULT_PROFILE;
    p.ambient_namespace_prefixes = &["az"];
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &p,
    };
    assert!(matches!(AmbientPrefixStripRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn passes_when_leaf_not_ambient() {
    // Prefix matches but the leaf is not in ambient scope.
    let lookup = Lookup::new();
    let r = call_ref("az.resourceId");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let mut p = DEFAULT_PROFILE;
    p.ambient_namespace_prefixes = &["az"];
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &p,
    };
    assert!(matches!(AmbientPrefixStripRule.apply(&ctx), LookupResult::Pass));
}
