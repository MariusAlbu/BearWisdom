use super::*;
use crate::indexer::resolve::engine::testkit::{
    accept_any, call_ref, file_ctx, ref_ctx, source_symbol, sym, Lookup,
};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{LanguageProfile, DEFAULT_PROFILE};
use crate::types::{EdgeKind, ExtractedRef, SymbolKind};

/// A profile whose functions and constants fall back to the root namespace and
/// whose source qualification separator is a backslash.
fn root_fallback_profile() -> LanguageProfile {
    LanguageProfile {
        root_namespace_fallback: &[SymbolKind::Function, SymbolKind::Field],
        qname_separator: "\\",
        ..DEFAULT_PROFILE
    }
}

/// `ExtractedRef` with a configurable edge kind, for the constant arm.
fn typed_ref(target: &str, kind: EdgeKind) -> ExtractedRef {
    ExtractedRef {
        is_include: false,
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

/// Only a `class` candidate is kind-compatible.
fn accept_class_only(_: EdgeKind, kind: &str) -> bool {
    kind == "class"
}

fn resolve_with(
    lookup: &Lookup,
    r: &ExtractedRef,
    ns: Option<&str>,
    profile: &LanguageProfile,
    kind: &dyn Fn(EdgeKind, &str) -> bool,
) -> Option<i64> {
    let s = source_symbol("pluck");
    let fc = file_ctx(vec![], ns);
    let rc = ref_ctx(r, &s, vec![]);
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup,
        kind,
        profile,
    };
    match RootNamespaceRule.apply(&ctx) {
        LookupResult::Resolved(res) => {
            assert_eq!(res.strategy, "root_namespace");
            Some(res.target_symbol_id)
        }
        _ => None,
    }
}

fn resolve(lookup: &Lookup, target: &str, ns: Option<&str>) -> Option<i64> {
    let r = call_ref(target);
    resolve_with(lookup, &r, ns, &root_fallback_profile(), &accept_any)
}

#[test]
fn binds_bare_call_to_root_function_from_namespaced_file() {
    // The file declares a namespace of its own; the callee owns no prefix, so
    // its stored qname IS the bare target.
    let lookup = Lookup::new().with(sym(10, "data_get", "data_get", "function", "src/helpers.php"));
    assert_eq!(
        resolve(&lookup, "data_get", Some("Illuminate\\Support")),
        Some(10)
    );
}

#[test]
fn binds_when_file_has_no_namespace() {
    // A root-namespace caller: the same-namespace rung declines it outright.
    let lookup = Lookup::new().with(sym(10, "data_get", "data_get", "function", "src/helpers.php"));
    assert_eq!(resolve(&lookup, "data_get", None), Some(10));
}

#[test]
fn declines_class_kind() {
    // The kind set is what keeps this axis data: a bare class name never falls
    // back to the root.
    let lookup = Lookup::new().with(sym(11, "Arr", "Arr", "class", "src/Arr.php"));
    assert_eq!(resolve(&lookup, "Arr", Some("Illuminate\\Support")), None);
}

#[test]
fn declines_qualified_target() {
    let lookup = Lookup::new().with(sym(
        11,
        "Arr",
        "Illuminate\\Support\\Arr",
        "function",
        "src/Arr.php",
    ));
    assert_eq!(
        resolve(&lookup, "Illuminate\\Support\\Arr", Some("App")),
        None
    );
}

#[test]
fn declines_when_axis_empty() {
    let lookup = Lookup::new().with(sym(10, "data_get", "data_get", "function", "src/helpers.php"));
    let r = call_ref("data_get");
    assert_eq!(
        resolve_with(&lookup, &r, None, &DEFAULT_PROFILE, &accept_any),
        None
    );
}

#[test]
fn declines_when_kind_table_rejects() {
    // The edge→kind gate still applies on top of the axis.
    let lookup = Lookup::new().with(sym(10, "data_get", "data_get", "function", "src/helpers.php"));
    let r = call_ref("data_get");
    assert_eq!(
        resolve_with(
            &lookup,
            &r,
            None,
            &root_fallback_profile(),
            &accept_class_only
        ),
        None
    );
}

#[test]
fn binds_sole_candidate_and_declines_ambiguous_pair() {
    let sole = Lookup::new().with(sym(10, "url", "url", "function", "src/helpers.php"));
    assert_eq!(resolve(&sole, "url", None), Some(10));

    // Two root declarations in unrelated directories score equally, so the
    // shared chokepoint declines rather than guessing a first match.
    let ambiguous = Lookup::new()
        .with(sym(10, "url", "url", "function", "one/helpers.php"))
        .with(sym(11, "url", "url", "function", "two/helpers.php"));
    assert_eq!(resolve(&ambiguous, "url", None), None);
}

#[test]
fn binds_root_constant_field() {
    let lookup = Lookup::new().with(sym(
        12,
        "PHP_INT_MAX",
        "PHP_INT_MAX",
        "field",
        "src/const.php",
    ));
    let r = typed_ref("PHP_INT_MAX", EdgeKind::Reads);
    assert_eq!(
        resolve_with(
            &lookup,
            &r,
            Some("Illuminate\\Support"),
            &root_fallback_profile(),
            &accept_any
        ),
        Some(12)
    );
}
