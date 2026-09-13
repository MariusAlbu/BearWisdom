use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::type_checker::profile::language_profile::{LanguageProfile, DEFAULT_PROFILE};
use crate::types::{EdgeKind, ExtractedRef};

/// Build an `ExtractedRef` for `target` with `module` set.
fn module_ref(target: &str, module: &str) -> ExtractedRef {
    ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: Some(module.to_string()),
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

static COLON_COLON_PROFILE: LanguageProfile = LanguageProfile {
    qname_separator: "::",
    ..DEFAULT_PROFILE
};

/// Kind predicate that refuses module declarations — the shape a language whose
/// KindTable omits `Module` for the ref's edge kind produces.
fn reject_module(_: EdgeKind, kind: &str) -> bool {
    kind != "module"
}

fn resolve_with(
    lookup: &Lookup,
    target: &str,
    module: &str,
    profile: &'static LanguageProfile,
    kind: &dyn Fn(EdgeKind, &str) -> bool,
) -> Option<i64> {
    let r = module_ref(target, module);
    let s = source_symbol("caller");
    let fc = file_ctx(vec![], None);
    let rc = ref_ctx(&r, &s, vec![]);
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind,
        profile,
    };
    match RefModuleRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

fn resolve_with_profile(
    lookup: &Lookup,
    target: &str,
    module: &str,
    profile: &'static LanguageProfile,
) -> Option<i64> {
    let kind = accept_any;
    resolve_with(lookup, target, module, profile, &kind)
}

fn resolve(lookup: &Lookup, target: &str, module: &str) -> Option<i64> {
    resolve_with_profile(lookup, target, module, &DEFAULT_PROFILE)
}

fn resolve_no_module(lookup: &Lookup, target: &str) -> Option<i64> {
    use crate::indexer::resolve::engine::testkit::call_ref;
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
    match RefModuleRule.apply(&ctx) {
        LookupResult::Resolved(res) => Some(res.target_symbol_id),
        _ => None,
    }
}

#[test]
fn binds_via_dot_qualified_name() {
    // `lists.map` → exact qname with `.` separator.
    let lookup = Lookup::new().with(sym(1, "map", "lists.map", "function", "src/lists.erl"));
    let got = resolve(&lookup, "map", "lists");
    assert_eq!(got, Some(1));
}

#[test]
fn source_separator_normalizes_to_canonical_index_qname() {
    let lookup = Lookup::new().with(sym(
        2,
        "mutate",
        "dplyr.verbs.mutate",
        "function",
        "R/dplyr.R",
    ));
    let got = resolve_with_profile(&lookup, "mutate", "dplyr::verbs", &COLON_COLON_PROFILE);
    assert_eq!(got, Some(2));
}

#[test]
fn file_stem_fallback_uses_profile_module_separator() {
    let lookup = Lookup::new().with(sym(22, "read", "read", "function", "src/io/reader.rs"));
    let got = resolve_with_profile(&lookup, "read", "crate::io::reader", &COLON_COLON_PROFILE);
    assert_eq!(got, Some(22));
}

#[test]
fn declines_when_no_module_set() {
    let lookup = Lookup::new().with(sym(3, "map", "lists.map", "function", "src/lists.erl"));
    let got = resolve_no_module(&lookup, "map");
    assert_eq!(got, None);
}

#[test]
fn binds_when_the_module_spelling_is_the_target_qname() {
    // `Plausible.Repo.all()` — the module spelling IS the declaration's qname and
    // the target is its leaf. The declaration's `name` is the whole dotted
    // string, so no by-name arm can reach it.
    let lookup = Lookup::new().with(sym(
        1,
        "Plausible.Repo",
        "Plausible.Repo",
        "module",
        "lib/plausible/repo.ex",
    ));
    let got = resolve(&lookup, "Repo", "Plausible.Repo");
    assert_eq!(got, Some(1));
}

#[test]
fn member_ref_under_the_same_module_still_takes_the_container_arm() {
    let lookup = Lookup::new()
        .with(sym(
            1,
            "Plausible.Repo",
            "Plausible.Repo",
            "module",
            "lib/plausible/repo.ex",
        ))
        .with(sym(
            2,
            "all",
            "Plausible.Repo.all",
            "function",
            "lib/plausible/repo.ex",
        ));
    let got = resolve(&lookup, "all", "Plausible.Repo");
    assert_eq!(got, Some(2));
}

#[test]
fn receiver_valued_module_never_binds_a_module_symbol() {
    // Languages that overload `module` with the chain receiver put an expression
    // there; its leaf is not the target, so the entity arm must decline.
    let lookup = Lookup::new().with(sym(
        7,
        "conn.assigns",
        "conn.assigns",
        "module",
        "lib/web/conn.ex",
    ));
    let got = resolve(&lookup, "flash", "conn.assigns");
    assert_eq!(got, None);
}

#[test]
fn entity_arm_respects_kind_compatibility() {
    let lookup = Lookup::new().with(sym(
        1,
        "Plausible.Repo",
        "Plausible.Repo",
        "module",
        "lib/plausible/repo.ex",
    ));
    let kind = reject_module;
    let got = resolve_with(&lookup, "Repo", "Plausible.Repo", &DEFAULT_PROFILE, &kind);
    assert_eq!(got, None);
}

#[test]
fn entity_arm_normalizes_the_source_separator() {
    let lookup = Lookup::new().with(sym(9, "dplyr.verbs", "dplyr.verbs", "module", "R/dplyr.R"));
    let got = resolve_with_profile(&lookup, "verbs", "dplyr::verbs", &COLON_COLON_PROFILE);
    assert_eq!(got, Some(9));
}

#[test]
fn declines_when_qname_not_found() {
    let lookup = Lookup::new().with(sym(
        4,
        "filter",
        "lists.filter",
        "function",
        "src/lists.erl",
    ));
    let got = resolve(&lookup, "map", "lists");
    assert_eq!(got, None);
}

#[test]
fn a_bare_module_equal_to_the_target_is_not_denotation_evidence() {
    // `import ast` records module `ast` under target `ast`; a project variable
    // spelled `ast` must not be bound through what would be an unguarded
    // bare-qname probe.
    let lookup = Lookup::new().with(sym(5, "ast", "ast", "variable", "src/tools.py"));
    let got = resolve(&lookup, "ast", "ast");
    assert_eq!(got, None);
}

#[test]
fn a_declaration_header_never_binds_to_itself() {
    // The `defmodule Plausible.Audit` header's own alias node references the
    // module it declares; binding it would record a self-loop edge.
    let lookup = Lookup::new().with(sym(
        1,
        "Plausible.Audit",
        "Plausible.Audit",
        "module",
        "lib/plausible/audit.ex",
    ));
    let r = module_ref("Audit", "Plausible.Audit");
    let s = source_symbol("Plausible.Audit");
    let fc = file_ctx(vec![], None);
    let mut rc = ref_ctx(&r, &s, vec![]);
    rc.source_symbol_id = Some(1);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    assert!(matches!(RefModuleRule.apply(&ctx), LookupResult::Pass));

    // The same spelling from another declaration binds.
    rc.source_symbol_id = Some(2);
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE,
    };
    match RefModuleRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 1),
        other => panic!("expected a bind, got {other:?}"),
    }
}
