use super::{FSHARP_KIND_TABLE, FSHARP_PROFILE};
use crate::type_checker::profile::language_profile::KindCompatibility;
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
