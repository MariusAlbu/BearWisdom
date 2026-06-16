use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{NamespaceScope, DEFAULT_PROFILE};

#[test]
fn passes_when_gate_is_off() {
    // DEFAULT_PROFILE has namespaceless_global_type_lookup = Off.
    let lookup = Lookup::new().with(sym(1, "users", "users", "table", "schema/users.sql"));
    let r = call_ref("users");
    let s = source_symbol("query");
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
    assert!(matches!(NamespacelessGlobalRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn resolves_bare_name_in_global_scope() {
    let lookup = Lookup::new().with(sym(10, "users", "users", "table", "schema/users.sql"));
    let r = call_ref("users");
    let s = source_symbol("query");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let mut p = DEFAULT_PROFILE;
    p.namespaceless_global_type_lookup = NamespaceScope::Global;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &p,
    };
    match NamespacelessGlobalRule.apply(&ctx) {
        LookupResult::Resolved(res) => {
            assert_eq!(res.target_symbol_id, 10);
            assert_eq!(res.strategy, "default_namespaceless_global");
        }
        _ => panic!("expected Resolved"),
    }
}

#[test]
fn directory_scoped_binds_same_dir_only() {
    // src/a.sql references `users`; only the candidate in `src/` should bind.
    let lookup = Lookup::new()
        .with(sym(20, "users", "users_other", "table", "other/users.sql"))
        .with(sym(21, "users", "users_src", "table", "src/users.sql"));
    let r = call_ref("users");
    let s = source_symbol("query");
    let fc = file_ctx(vec![], None); // file_ctx uses "src/main.ts" by default
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let mut p = DEFAULT_PROFILE;
    p.namespaceless_global_type_lookup = NamespaceScope::DirectoryScoped;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &p,
    };
    match NamespacelessGlobalRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 21),
        _ => panic!("expected Resolved to src/users.sql candidate"),
    }
}

#[test]
fn strips_self_keyword_prefix_before_lookup() {
    // `var.users` → `users` after stripping `var` self keyword.
    let lookup = Lookup::new().with(sym(30, "users", "users", "table", "schema/users.sql"));
    let r = call_ref("var.users");
    let s = source_symbol("query");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let mut p = DEFAULT_PROFILE;
    p.namespaceless_global_type_lookup = NamespaceScope::Global;
    p.self_keywords = &["var"];
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &p,
    };
    match NamespacelessGlobalRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 30),
        _ => panic!("expected Resolved after self-keyword strip"),
    }
}

#[test]
fn skips_external_candidates() {
    // A symbol whose path starts with `ext:` is external and must not bind.
    let lookup = Lookup::new()
        .with(sym(40, "len", "len", "function", "ext:py-stdlib:builtins.py"))
        .with(sym(41, "len", "len", "function", "src/utils.sql"));
    let r = call_ref("len");
    let s = source_symbol("query");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let mut p = DEFAULT_PROFILE;
    p.namespaceless_global_type_lookup = NamespaceScope::Global;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &p,
    };
    // id 40 is external (ext: prefix), id 41 is internal; must bind 41.
    match NamespacelessGlobalRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 41),
        _ => panic!("expected Resolved to internal candidate"),
    }
}
