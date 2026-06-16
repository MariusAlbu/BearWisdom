use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;

fn resolve(lookup: &Lookup, target: &str, scope_chain: Vec<String>) -> Option<i64> {
    let r = call_ref(target);
    let s = source_symbol("method");
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
    match EnclosingMemberRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_member_of_enclosing_class() {
    // `Foo` is the enclosing class (in scope chain) and has a `doWork` member.
    let lookup = Lookup::new()
        .with(sym(1, "Foo", "Foo", "class", "src/foo.ts"))
        .with_member("Foo", sym(2, "doWork", "Foo.doWork", "function", "src/foo.ts"));
    let got = resolve(&lookup, "doWork", vec!["Foo".to_string()]);
    assert_eq!(got, Some(2));
}

#[test]
fn declines_dotted_target() {
    let lookup = Lookup::new()
        .with(sym(1, "Foo", "Foo", "class", "src/foo.ts"))
        .with_member("Foo", sym(2, "doWork", "Foo.doWork", "function", "src/foo.ts"));
    // A dotted target is not an inherited-member candidate.
    let got = resolve(&lookup, "Foo.doWork", vec!["Foo".to_string()]);
    assert_eq!(got, None);
}

#[test]
fn declines_when_no_enclosing_type_in_scope() {
    // Scope chain only has a function-kind — enclosing_type returns None.
    let lookup = Lookup::new()
        .with(sym(3, "free", "free", "function", "src/a.ts"))
        .with_member("free", sym(4, "x", "free.x", "function", "src/a.ts"));
    let got = resolve(&lookup, "x", vec!["free".to_string()]);
    assert_eq!(got, None);
}

#[test]
fn declines_when_member_not_found() {
    let lookup = Lookup::new()
        .with(sym(1, "Foo", "Foo", "class", "src/foo.ts"));
    // No members registered for Foo.
    let got = resolve(&lookup, "missing", vec!["Foo".to_string()]);
    assert_eq!(got, None);
}
