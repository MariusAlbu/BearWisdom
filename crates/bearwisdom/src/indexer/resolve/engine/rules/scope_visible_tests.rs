use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

/// Build a `BinderContext` over the given pieces and apply `ScopeVisibleRule`.
fn resolve(lookup: &Lookup, target: &str, scope_chain: Vec<String>) -> Option<i64> {
    let r = call_ref(target);
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, scope_chain);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    match ScopeVisibleRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_member_of_innermost_scope() {
    let lookup = Lookup::new().with(sym(42, "helper", "Foo.Bar.helper", "function", "src/a.ts"));
    let got = resolve(&lookup, "helper", vec!["Foo.Bar".to_string(), "Foo".to_string()]);
    assert_eq!(got, Some(42));
}

#[test]
fn innermost_scope_wins_over_outer() {
    let lookup = Lookup::new()
        .with(sym(1, "x", "Outer.x", "function", "src/a.ts"))
        .with(sym(2, "x", "Outer.Inner.x", "function", "src/a.ts"));
    let got = resolve(
        &lookup,
        "x",
        vec!["Outer.Inner".to_string(), "Outer".to_string()],
    );
    assert_eq!(got, Some(2));
}

#[test]
fn declines_when_no_scope_matches() {
    let lookup = Lookup::new().with(sym(7, "helper", "Other.helper", "function", "src/a.ts"));
    let got = resolve(&lookup, "helper", vec!["Foo.Bar".to_string()]);
    assert_eq!(got, None);
}

#[test]
fn declines_with_empty_scope_chain() {
    let lookup = Lookup::new().with(sym(7, "helper", "Foo.helper", "function", "src/a.ts"));
    let got = resolve(&lookup, "helper", vec![]);
    assert_eq!(got, None);
}
