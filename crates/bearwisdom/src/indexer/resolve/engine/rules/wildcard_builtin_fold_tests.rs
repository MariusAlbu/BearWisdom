use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{WildcardBuiltin, DEFAULT_PROFILE};

const LIST_BUILTIN: WildcardBuiltin = WildcardBuiltin {
    prefix: "list",
    fold_to: "list",
};

#[test]
fn passes_when_no_wildcard_builtins_configured() {
    // DEFAULT_PROFILE has wildcard_builtins = &[], so always Pass.
    let lookup = Lookup::new();
    let r = call_ref("listConnectionStrings");
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
    assert!(matches!(WildcardBuiltinFoldRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn folds_and_resolves_ambient_symbol() {
    // `listConnectionStrings` folds to `list` via the builtin, then the
    // ambient-scope lookup finds `list` flagged ambient.
    let lookup = Lookup::new().with_ambient(sym(
        42,
        "list",
        "list",
        "function",
        "node_modules/@types/azure/index.d.ts",
    ));
    let r = call_ref("listConnectionStrings");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let mut p = DEFAULT_PROFILE;
    p.wildcard_builtins = &[LIST_BUILTIN];
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &p,
    };
    match WildcardBuiltinFoldRule.apply(&ctx) {
        LookupResult::Resolved(res) => {
            assert_eq!(res.target_symbol_id, 42);
            assert_eq!(res.strategy, "wildcard_builtin_fold");
        }
        _ => panic!("expected Resolved"),
    }
}

#[test]
fn passes_when_target_does_not_match_any_builtin() {
    // `getConnectionString` does not start with `list` + uppercase, so no fold.
    let lookup = Lookup::new().with_ambient(sym(
        99,
        "list",
        "list",
        "function",
        "node_modules/@types/azure/index.d.ts",
    ));
    let r = call_ref("getConnectionString");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let mut p = DEFAULT_PROFILE;
    p.wildcard_builtins = &[LIST_BUILTIN];
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &p,
    };
    assert!(matches!(WildcardBuiltinFoldRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn passes_when_fold_target_not_ambient() {
    // Fold succeeds but the `list` symbol is not in ambient scope — Pass.
    let lookup = Lookup::new();
    let r = call_ref("listKeys");
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let mut p = DEFAULT_PROFILE;
    p.wildcard_builtins = &[LIST_BUILTIN];
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &p,
    };
    assert!(matches!(WildcardBuiltinFoldRule.apply(&ctx), LookupResult::Pass));
}
