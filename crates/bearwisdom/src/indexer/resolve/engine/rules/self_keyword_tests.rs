use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

/// Build a `BinderContext` and apply `SelfKeywordRule`.
fn resolve(
    lookup: &Lookup,
    target: &str,
    scope_chain: Vec<String>,
) -> Option<i64> {
    let r = call_ref(target);
    let s = source_symbol("SomeMethod");
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
    match SelfKeywordRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_this_to_enclosing_class_via_scope_chain() {
    // The enclosing class is in the scope chain; `this` binds to it.
    let lookup = Lookup::new()
        .with(sym(5, "Foo", "Foo", "class", "src/foo.ts"));
    let got = resolve(&lookup, "this", vec!["Foo".to_string()]);
    assert_eq!(got, Some(5));
}

#[test]
fn binds_self_to_enclosing_struct() {
    let lookup = Lookup::new()
        .with(sym(7, "MyStruct", "MyStruct", "struct", "src/lib.rs"));
    let got = resolve(&lookup, "self", vec!["MyStruct".to_string()]);
    assert_eq!(got, Some(7));
}

#[test]
fn declines_for_non_keyword_target() {
    let lookup = Lookup::new()
        .with(sym(9, "Foo", "Foo", "class", "src/foo.ts"));
    let got = resolve(&lookup, "someFunction", vec!["Foo".to_string()]);
    assert_eq!(got, None);
}

#[test]
fn declines_when_no_enclosing_type_in_scope_chain() {
    // The scope chain holds a function-kind scope, not a type — enclosing_type
    // returns None, so the rule passes.
    let lookup = Lookup::new()
        .with(sym(3, "freeFunc", "freeFunc", "function", "src/a.ts"));
    let got = resolve(&lookup, "this", vec!["freeFunc".to_string()]);
    assert_eq!(got, None);
}

#[test]
fn binds_super_to_direct_parent_by_id() {
    // `Child extends Base`; `super` binds Base by the id-keyed direct-parent edge,
    // not a first-winner qname re-search.
    let lookup = Lookup::new()
        .with(sym(10, "Child", "Child", "class", "src/child.ts"))
        .with(sym(20, "Base", "Base", "class", "src/base.ts"))
        .with_parent_id(10, 20);
    let got = resolve(&lookup, "super", vec!["Child".to_string()]);
    assert_eq!(got, Some(20));
}
