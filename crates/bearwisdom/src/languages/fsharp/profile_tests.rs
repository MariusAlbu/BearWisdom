use super::{FSHARP_KIND_TABLE, FSHARP_PROFILE};
use crate::type_checker::profile::language_profile::{ImportModulePath, KindCompatibility};
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn fsharp_profile_identity_and_shadow_mode() {
    assert_eq!(FSHARP_PROFILE.id, "fsharp");
    assert!(FSHARP_PROFILE.async_wrappers.contains(&"Async"));
    assert!(FSHARP_PROFILE.async_wrappers.contains(&"Task"));
}

#[test]
fn union_case_application_is_callable() {
    // `Ok value` / `Circle`: a discriminated-union case extracted as
    // `EnumMember`, applied as a `Calls` ref, must be a compatible target.
    assert!(KindCompatibility::check(
        FSHARP_KIND_TABLE,
        EdgeKind::Calls,
        SymbolKind::EnumMember,
    ));
}

#[test]
fn ordinary_function_call_unaffected() {
    assert!(KindCompatibility::check(
        FSHARP_KIND_TABLE,
        EdgeKind::Calls,
        SymbolKind::Function,
    ));
}

#[test]
fn union_case_is_not_a_type_target() {
    // The widening is `Calls`-only: a case is not a `TypeRef` target.
    assert!(!KindCompatibility::check(
        FSHARP_KIND_TABLE,
        EdgeKind::TypeRef,
        SymbolKind::EnumMember,
    ));
}

#[test]
fn open_declarations_are_wildcard_imports() {
    // `open Foo` has no named-member form — every open brings the whole
    // namespace into bare scope, so the profile must treat plain imports as
    // wildcards and source their module path from the ref's `module` field
    // (which `extract_open` / `extract_hash_r_directives` always set).
    assert!(FSHARP_PROFILE.namespace_imports_are_wildcards);
    assert_eq!(FSHARP_PROFILE.import_module_path, ImportModulePath::FromModuleField);
}

#[test]
fn builtin_skip_is_wired_to_the_prelude_operator_predicate() {
    let skip = FSHARP_PROFILE.builtin_skip.expect("F# drains prelude operators");
    assert!(skip("sprintf"));
    assert!(!skip("Some"));
}
