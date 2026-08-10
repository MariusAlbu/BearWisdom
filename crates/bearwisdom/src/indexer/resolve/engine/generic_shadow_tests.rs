use super::*;
use crate::indexer::resolve::engine::chain::bind_member_access;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::core::types::{GenericParamData, Type};
use crate::type_checker::profile::language_profile::{LanguageProfile, DEFAULT_PROFILE};
use crate::types::{ChainSegment, EdgeKind, MemberChain, SegmentKind};

// ---------------------------------------------------------------------------
// GenericParamShadowRule — the resolution-ladder drain guard
// ---------------------------------------------------------------------------

fn shadow_ctx_verdict(
    lookup: &Lookup,
    kind: EdgeKind,
    target: &str,
    source_qname: &str,
) -> LookupResult {
    let mut r = call_ref(target);
    r.kind = kind;
    let mut s = source_symbol("src");
    s.qualified_name = source_qname.to_string();
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind: &accept_any,
        profile: &DEFAULT_PROFILE,
    };
    GenericParamShadowRule.apply(&ctx)
}

/// `Resolvers.Sync<TSource, T>(...)` mentions `T` in a parameter type: the
/// type_ref must drain instead of binding the indexed concrete class `T`.
#[test]
fn a_type_ref_naming_an_own_method_param_drains() {
    let lookup = Lookup::new()
        .with(sym(30, "T", "App.Translations.T", "class", "src/T.cs"))
        .with_generics("App.Resolvers.Sync", &["TSource", "T"]);
    let verdict = shadow_ctx_verdict(&lookup, EdgeKind::TypeRef, "T", "App.Resolvers.Sync");
    assert!(matches!(verdict, LookupResult::Drained), "in-scope param must drain");
}

/// The same target from a NON-generic method passes through, so the ladder's
/// later rungs still bind the concrete class normally.
#[test]
fn the_same_type_ref_from_a_non_generic_method_passes() {
    let lookup = Lookup::new()
        .with(sym(30, "T", "App.Translations.T", "class", "src/T.cs"))
        .with_generics("App.Resolvers.Sync", &["TSource", "T"]);
    let verdict = shadow_ctx_verdict(&lookup, EdgeKind::TypeRef, "T", "App.Resolvers.Plain");
    assert!(matches!(verdict, LookupResult::Pass), "no param in scope — pass through");
}

/// A parameter of an ENCLOSING type is in scope for a member's body: the
/// prefix walk sees `Repo<T>`'s `T` from `App.Repo.Find`.
#[test]
fn an_enclosing_type_param_also_vetoes() {
    let lookup = Lookup::new()
        .with(sym(30, "T", "App.Translations.T", "class", "src/T.cs"))
        .with_generics("App.Repo", &["T"]);
    let verdict = shadow_ctx_verdict(&lookup, EdgeKind::TypeRef, "T", "App.Repo.Find");
    assert!(matches!(verdict, LookupResult::Drained), "owner param must drain");
}

#[test]
fn an_instantiates_ref_of_an_in_scope_param_drains() {
    let lookup = Lookup::new().with_generics("App.Factory.Create", &["T"]);
    let verdict = shadow_ctx_verdict(&lookup, EdgeKind::Instantiates, "T", "App.Factory.Create");
    assert!(matches!(verdict, LookupResult::Drained));
}

/// The veto is scoped to nominal-binding kinds: a call named like a param is
/// not a type mention and must reach the ordinary rungs.
#[test]
fn a_calls_ref_is_not_vetoed() {
    let lookup = Lookup::new().with_generics("App.Resolvers.Sync", &["T"]);
    let verdict = shadow_ctx_verdict(&lookup, EdgeKind::Calls, "T", "App.Resolvers.Sync");
    assert!(matches!(verdict, LookupResult::Pass));
}

#[test]
fn a_dotted_target_is_not_a_param_mention() {
    let lookup = Lookup::new().with_generics("App.Resolvers.Sync", &["T"]);
    let verdict = shadow_ctx_verdict(&lookup, EdgeKind::TypeRef, "Ns.T", "App.Resolvers.Sync");
    assert!(matches!(verdict, LookupResult::Pass));
}

/// Params carried on the source symbol itself (arena-interned at extract
/// time) veto without any qname-keyed slot.
#[test]
fn arena_interned_source_params_also_veto() {
    let lookup = Lookup::new().with(sym(30, "T", "App.Translations.T", "class", "src/T.cs"));
    let gp = lookup.type_arena().unwrap().intern_generic(GenericParamData {
        name: "T".to_string(),
        owner_symbol_index: 0,
        bound: None,
    });
    let mut r = call_ref("T");
    r.kind = EdgeKind::TypeRef;
    let mut s = source_symbol("Sync");
    s.qualified_name = "App.Resolvers.Sync".to_string();
    s.generic_params = vec![gp];
    let rc = ref_ctx(&r, &s, vec![]);
    let fc = file_ctx(vec![], None);
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &accept_any,
        profile: &DEFAULT_PROFILE,
    };
    assert!(matches!(GenericParamShadowRule.apply(&ctx), LookupResult::Drained));
}

// ---------------------------------------------------------------------------
// mark_unbound_member_params — the member-yield rewrite
// ---------------------------------------------------------------------------

#[test]
fn a_declaring_type_param_left_in_the_yield_becomes_a_generic_marker() {
    let lookup = Lookup::new().with_generics("NamedId", &["T"]);
    let arena = lookup.type_arena().unwrap();
    let member = sym(11, "Id", "NamedId.Id", "property", "src/NamedId.cs");
    let yielded = arena.intern_type_str("T");
    let marked = mark_unbound_member_params(&lookup, arena, &member, yielded);
    assert_ne!(marked, yielded, "the nominal param leaf must be rewritten");
    assert!(matches!(arena.get(marked), Type::Generic { .. }));
}

#[test]
fn a_method_own_param_inside_an_application_becomes_a_generic_marker() {
    let lookup = Lookup::new().with_generics("App.Resolvers.Sync", &["TSource", "T"]);
    let arena = lookup.type_arena().unwrap();
    let member = sym(40, "Sync", "App.Resolvers.Sync", "method", "src/R.cs");
    let yielded = arena.intern_type_str("Func<TSource, T>");
    let marked = mark_unbound_member_params(&lookup, arena, &member, yielded);
    match arena.get(marked) {
        Type::Apply { args, .. } => {
            assert_eq!(args.len(), 2);
            for a in args {
                assert!(matches!(arena.get(a), Type::Generic { .. }), "each param arg is marked");
            }
        }
        other => panic!("expected an application, got {other:?}"),
    }
}

#[test]
fn a_concrete_yield_is_untouched() {
    let lookup = Lookup::new().with_generics("NamedId", &["T"]);
    let arena = lookup.type_arena().unwrap();
    let member = sym(11, "Id", "NamedId.Id", "property", "src/NamedId.cs");
    let yielded = arena.intern_type_str("DomainId");
    assert_eq!(mark_unbound_member_params(&lookup, arena, &member, yielded), yielded);
}

// ---------------------------------------------------------------------------
// End-to-end chain behavior: `nid.Id.ToString()` where `NamedId<T>.Id : T`
// ---------------------------------------------------------------------------

static ROOTED: LanguageProfile = LanguageProfile {
    implicit_root_types: &["Object"],
    ..DEFAULT_PROFILE
};

const ROOT_FILE: &str = "ext:dotnet-type:CoreLib.dll!!System!!Object";

fn seg(name: &str, is_call: bool) -> ChainSegment {
    ChainSegment {
        name: name.to_string(),
        node_kind: String::new(),
        kind: SegmentKind::Identifier,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    }
}

/// The receiver value plus the generic declaring type, the implicit root, and
/// its `ToString` — the shared stage for both chain tests.
fn nid_lookup(receiver_field_type: &str) -> Lookup {
    Lookup::new()
        .with(sym(1, "nid", "M.nid", "parameter", "src/M.cs"))
        .with_field_type("M.nid", receiver_field_type)
        .with(sym(10, "NamedId", "NamedId", "class", "src/NamedId.cs"))
        .with_generics("NamedId", &["T"])
        .with_member("NamedId", sym(11, "Id", "NamedId.Id", "property", "src/NamedId.cs"))
        .with_field_type("NamedId.Id", "T")
        .with(sym(90, "Object", "System.Object", "class", ROOT_FILE))
        .with_member(
            "System.Object",
            sym(91, "ToString", "System.Object.ToString", "method", ROOT_FILE),
        )
}

fn resolve_tostring(lookup: &Lookup) -> Option<i64> {
    let segs = vec![seg("nid", false), seg("Id", false), seg("ToString", true)];
    let mut r = call_ref("ToString");
    r.chain = Some(MemberChain { segments: segs });
    let mut s = source_symbol("caller");
    s.qualified_name = "M.Run".to_string();
    let rc = ref_ctx(&r, &s, vec![]);
    bind_member_access(&rc, &file_ctx(vec![], None), lookup, &ROOTED)
        .map(|i| i.target_symbol_id)
        .ok()
}

/// Receiver `NamedId<DomainId>`: substitution binds `T → DomainId`, and the
/// final hop resolves on the concrete argument — the applied path is intact.
#[test]
fn an_applied_receiver_still_substitutes_and_resolves() {
    let lookup = nid_lookup("NamedId<DomainId>")
        .with(sym(20, "DomainId", "DomainId", "class", "src/DomainId.cs"))
        .with_member(
            "DomainId",
            sym(21, "ToString", "DomainId.ToString", "method", "src/DomainId.cs"),
        );
    assert_eq!(resolve_tostring(&lookup), Some(21));
}

/// Receiver `NamedId` with NO applied args: `.Id` yields an unbound `T`. A
/// concrete class named `T` is indexed and the profile closes walks at
/// `Object`, whose `ToString` is indexed — yet the hop must stay a miss:
/// the marker has no nominal head, so the same-named class is never reheaded
/// onto and the implicit-root gate (unbound receiver) keeps the miss.
#[test]
fn an_unapplied_receiver_does_not_bind_a_same_named_concrete_class() {
    let lookup = nid_lookup("NamedId").with(sym(30, "T", "Lib.T", "class", "src/T.cs"));
    assert_eq!(
        resolve_tostring(&lookup),
        None,
        "an unbound generic yield must not resolve through a concrete `T` or the root"
    );
}
