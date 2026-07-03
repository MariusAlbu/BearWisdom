use super::*;
use crate::indexer::resolve::engine::testkit::{accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

fn resolve(lookup: &Lookup, target: &str) -> Option<i64> {
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
        profile: &DEFAULT_PROFILE,
    };
    match LocalFlowHeadRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_bare_call_through_the_local_callable_head_cache() {
    // `const { info } = makeLogger()` seeded `info`'s callable-head pointer to
    // the synthesized member's own qname (see
    // `chain::callable_member_qname_on`); a later bare call resolves through it.
    let lookup = Lookup::new()
        .with(sym(1, "info", "makeLogger$Ret.info", "property", "a.ts"))
        .with_local_callable_head("info", "makeLogger$Ret.info");
    assert_eq!(resolve(&lookup, "info"), Some(1));
}

#[test]
fn declines_when_no_callable_head_is_recorded() {
    let lookup = Lookup::new();
    assert_eq!(resolve(&lookup, "info"), None);
}

#[test]
fn declines_when_the_callable_head_names_no_symbol() {
    let lookup = Lookup::new().with_local_callable_head("info", "SomeUncapturedType.info");
    assert_eq!(resolve(&lookup, "info"), None);
}

#[test]
fn declines_when_the_named_symbol_is_kind_incompatible() {
    // The Calls edge accepts only callable-ish kinds; a callable-head pointer
    // to an interface must not bind — `accept_any` is swapped for a real
    // kind gate.
    let lookup = Lookup::new()
        .with(sym(2, "info", "SomeInterface", "interface", "a.ts"))
        .with_local_callable_head("info", "SomeInterface");
    let r = call_ref("info");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = |_: crate::types::EdgeKind, k: &str| k != "interface";
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    assert!(matches!(LocalFlowHeadRule.apply(&ctx), LookupResult::Pass));
}
