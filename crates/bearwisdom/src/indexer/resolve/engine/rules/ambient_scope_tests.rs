use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
use crate::types::EdgeKind;

/// `ExtractedRef` with a configurable edge kind, for the kind-gate test.
fn typed_ref(target: &str, kind: EdgeKind) -> crate::types::ExtractedRef {
    use crate::types::ExtractedRef;
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

/// A bare reference to a name flagged ambient binds to that symbol — with no
/// profile gate (DEFAULT_PROFILE has `ambient_globals = Off`), proving the rung
/// is language-agnostic.
#[test]
fn binds_bare_name_to_ambient_symbol() {
    let lookup = Lookup::new().with_ambient(sym(
        7,
        "expect",
        "expect",
        "function",
        "ext:ts:__npm_globals__/vitest.d.ts",
    ));
    let r = call_ref("expect");
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
    match AmbientScopeRule.apply(&ctx) {
        LookupResult::Resolved(res) => {
            assert_eq!(res.target_symbol_id, 7);
            assert_eq!(res.strategy, "ambient_scope");
        }
        other => panic!("expected Resolved, got {other:?}"),
    }
}

/// A bare name that is not in ambient scope declines — honestly unresolved.
#[test]
fn passes_when_not_ambient() {
    let lookup = Lookup::new();
    let r = call_ref("expect");
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
    assert!(matches!(AmbientScopeRule.apply(&ctx), LookupResult::Pass));
}

/// A dotted target is a member chain, not a bare ambient name — the rung
/// declines so an earlier chain rung owns it.
#[test]
fn passes_for_dotted_target() {
    let lookup = Lookup::new().with_ambient(sym(
        8,
        "expect",
        "expect",
        "function",
        "ext:ts:__npm_globals__/vitest.d.ts",
    ));
    let r = call_ref("obj.expect");
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
    assert!(matches!(AmbientScopeRule.apply(&ctx), LookupResult::Pass));
}

/// A kind-incompatible ambient candidate declines — a `Calls` ref against a
/// `variable` under a strict function-only predicate gets no relaxation, so the
/// rung passes rather than binding the wrong kind.
#[test]
fn respects_kind_predicate() {
    let lookup = Lookup::new().with_ambient(sym(
        9,
        "Widget",
        "Widget",
        "variable",
        "ext:ts:__ts_lib__/lib.dom.d.ts",
    ));
    let r = typed_ref("Widget", EdgeKind::Calls);
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = |_: EdgeKind, k: &str| k == "function";
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    assert!(matches!(AmbientScopeRule.apply(&ctx), LookupResult::Pass));
}

/// `new X()` against a `variable`-kind ambient symbol binds even under a strict
/// class-only predicate — core-lib constructors are `declare var X: { new(): Y }`.
#[test]
fn instantiate_relaxes_variable_kind() {
    let lookup = Lookup::new().with_ambient(sym(
        11,
        "Map",
        "Map",
        "variable",
        "ext:ts:__ts_lib__/lib.es2015.collection.d.ts",
    ));
    let r = typed_ref("Map", EdgeKind::Instantiates);
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = |_: EdgeKind, k: &str| k == "class";
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    match AmbientScopeRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 11),
        other => panic!("expected Resolved via instantiate relaxation, got {other:?}"),
    }
}
