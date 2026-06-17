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
    for canonical in [
        "string", "number", "boolean", "void", "never", "unknown", "any",
    ] {
        assert!(
            names.contains(&canonical),
            "TS primitive `{canonical}` missing from profile.primitive_mapping"
        );
    }
}

#[test]
fn calls_kind_table_accepts_function_method_variable() {
    let table = TYPESCRIPT_PROFILE.kind_compatible_table;
    for kind in [
        SymbolKind::Function,
        SymbolKind::Method,
        SymbolKind::Variable,
    ] {
        assert!(
            KindCompatibility::check(table, EdgeKind::Calls, kind),
            "Calls must accept {kind:?}"
        );
    }
}

#[test]
fn typeref_kind_table_accepts_type_and_import_binding_kinds() {
    let table = TYPESCRIPT_PROFILE.kind_compatible_table;
    // Type kinds are TypeRef targets.
    for kind in [
        SymbolKind::Class,
        SymbolKind::Interface,
        SymbolKind::Enum,
        SymbolKind::TypeAlias,
    ] {
        assert!(
            KindCompatibility::check(table, EdgeKind::TypeRef, kind),
            "{kind:?} IS a TypeRef target in TS"
        );
    }
    // The TS extractor emits every `import { X } from '...'` binding as a
    // TypeRef regardless of X's actual kind, so a function / variable /
    // namespace import must bind through TypeRef too.
    for kind in [
        SymbolKind::Function,
        SymbolKind::Variable,
        SymbolKind::Namespace,
        // The extractor emits `namespace X {}` / `declare namespace X` as
        // `Module`, so a namespace root (`Reflect.set`) binds through TypeRef.
        SymbolKind::Module,
    ] {
        assert!(
            KindCompatibility::check(table, EdgeKind::TypeRef, kind),
            "{kind:?} import binding must resolve through a TypeRef ref"
        );
    }
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
