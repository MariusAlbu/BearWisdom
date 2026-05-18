// =============================================================================
// languages/typescript/profile_tests.rs — TypeScript profile sanity checks.
// =============================================================================

use super::*;
use crate::type_checker::profile::language_profile::{
    DispatchAxis, KindCompatibility, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn id_matches_language_extractor() {
    assert_eq!(TYPESCRIPT_PROFILE.id, "typescript");
}

#[test]
fn structural_extremes_match_ts_semantics() {
    assert_eq!(
        TYPESCRIPT_PROFILE.supertype_discovery,
        SupertypeDiscovery::Both,
        "TS interfaces are structural, classes nominal — engine needs Both"
    );
    assert_eq!(TYPESCRIPT_PROFILE.dispatch_axis, DispatchAxis::Receiver);
    assert!(TYPESCRIPT_PROFILE.has_generics);
    assert!(TYPESCRIPT_PROFILE.has_sum_types);
    assert!(TYPESCRIPT_PROFILE.look_through_optional);
}

#[test]
fn promise_is_recognised_async_wrapper() {
    assert!(TYPESCRIPT_PROFILE
        .async_wrappers
        .iter()
        .any(|w| *w == "Promise"));
}

#[test]
fn self_keywords_cover_this() {
    assert!(TYPESCRIPT_PROFILE
        .self_keywords
        .iter()
        .any(|kw| *kw == "this"));
}

#[test]
fn primitives_include_canonical_ts_surface_types() {
    let names: Vec<&str> = TYPESCRIPT_PROFILE
        .primitive_mapping
        .iter()
        .map(|(n, _)| *n)
        .collect();
    for canonical in ["string", "number", "boolean", "void", "never", "unknown", "any"] {
        assert!(
            names.contains(&canonical),
            "TS primitive `{canonical}` missing from profile.primitive_mapping"
        );
    }
}

#[test]
fn calls_kind_table_accepts_function_method_variable() {
    let table = TYPESCRIPT_PROFILE.kind_compatible_table;
    for kind in [SymbolKind::Function, SymbolKind::Method, SymbolKind::Variable] {
        assert!(
            KindCompatibility::check(table, EdgeKind::Calls, kind),
            "Calls must accept {kind:?}"
        );
    }
}

#[test]
fn typeref_kind_table_rejects_value_kinds() {
    let table = TYPESCRIPT_PROFILE.kind_compatible_table;
    assert!(
        !KindCompatibility::check(table, EdgeKind::TypeRef, SymbolKind::Function),
        "Function is not a TypeRef target in TS"
    );
    assert!(
        !KindCompatibility::check(table, EdgeKind::TypeRef, SymbolKind::Variable),
        "Variable is not a TypeRef target in TS"
    );
    assert!(
        KindCompatibility::check(table, EdgeKind::TypeRef, SymbolKind::Class),
        "Class IS a TypeRef target in TS"
    );
    assert!(
        KindCompatibility::check(table, EdgeKind::TypeRef, SymbolKind::Interface),
        "Interface IS a TypeRef target in TS"
    );
}

#[test]
fn implements_kind_table_only_accepts_interface_and_type_alias() {
    let table = TYPESCRIPT_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(
        table,
        EdgeKind::Implements,
        SymbolKind::Interface
    ));
    assert!(KindCompatibility::check(
        table,
        EdgeKind::Implements,
        SymbolKind::TypeAlias
    ));
    assert!(!KindCompatibility::check(
        table,
        EdgeKind::Implements,
        SymbolKind::Class
    ));
}
