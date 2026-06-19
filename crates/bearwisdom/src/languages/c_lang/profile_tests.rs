use super::predicates;
use super::C_LANG_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn c_profile_identity_and_shadow_mode() {
    assert_eq!(C_LANG_PROFILE.id, "c");
}

/// `Calls → Variable` is admitted: C invokes callables through Variable symbols
/// (function pointers and object-like `#define` aliases of a callable). Function
/// and Method stay admitted; an Enum target stays refused, so the widening is
/// confined to the kind C legitimately calls through.
#[test]
fn c_calls_admits_variable_for_fnptr_and_macro_alias() {
    let t = C_LANG_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Variable
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Function
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Method
    ));
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Enum
    ));
}

/// A `TypeRef` to a Variable stays refused even though `Calls → Variable` is now
/// admitted — the Variable widening is scoped to the Calls row alone.
#[test]
fn c_typeref_still_refuses_variable() {
    let t = C_LANG_PROFILE.kind_compatible_table;
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Variable
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Struct
    ));
}

/// The plugin's `keywords()` exposes the full C/C++/POSIX spec set (the data
/// the resolver's external classifier consumes), not the 12-entry primitive
/// stub. Stdlib callees decline as builtins through this set.
#[test]
fn c_keywords_redirect_exposes_stdlib_set() {
    let kw = crate::indexer::keywords::keywords_for_language("c");
    assert!(
        kw.contains(&"strlen"),
        "keywords() must expose the full spec set (strlen)"
    );
    assert!(kw.contains(&"malloc"));
    assert!(kw.contains(&"fprintf"));
    // The purged nlohmann/json template-param name must not survive as a keyword.
    assert!(
        !kw.contains(&"BasicJsonType"),
        "project-specific template-param names must be purged from keywords()"
    );
    // `cpp` routes to the same plugin and therefore the same set.
    assert!(crate::indexer::keywords::keywords_for_language("cpp").contains(&"strlen"));
}

/// Purged template-param names stay suppressed through the generic
/// `is_template_param` predicate, which covers the patterns they matched
/// (the `EndsWith("Type")` rule and the `<Word>T` convention).
#[test]
fn purged_template_params_covered_by_predicate() {
    // `EndsWith("Type")` rule.
    assert!(predicates::is_template_param("BasicJsonType"));
    assert!(predicates::is_template_param("IteratorType"));
    assert!(predicates::is_template_param("AllocatorType"));
    assert!(predicates::is_template_param("ValueType"));
    assert!(predicates::is_template_param("ConstructibleArrayType"));
    // `<Word>T` convention.
    assert!(predicates::is_template_param("LhsT"));
    assert!(predicates::is_template_param("RhsT"));
    assert!(predicates::is_template_param("ArgT"));
    // The convention must not swallow ordinary type names that merely end in `T`
    // (all-caps acronyms / shouty constants are not `<Word>T`).
    assert!(!predicates::is_template_param("SAX"));
    assert!(!predicates::is_template_param("UINT"));
}

#[test]
fn c_profile_namespace_decline_gates_r_c_api() {
    // The R-package C-API decline is namespace-gated profile data: armed by the
    // R-package file namespace, reserves the R C API symbol set.
    let nd = C_LANG_PROFILE
        .namespace_decline
        .expect("c profile declares a namespace decline");
    assert_eq!(
        nd.file_namespace,
        predicates::R_PACKAGE_SENTINEL
    );
    assert!((nd.is_reserved)("Rf_eval"));
    assert!(!(nd.is_reserved)("my_project_fn"));
}
