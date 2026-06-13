use super::{HASKELL_KIND_TABLE, HASKELL_PROFILE};
use crate::type_checker::profile::language_profile::{DispatchAxis, KindCompatibility};
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn haskell_profile_identity() {
    assert_eq!(HASKELL_PROFILE.id, "haskell");
}

#[test]
fn haskell_dispatch_axis_is_return_type() {
    assert_eq!(HASKELL_PROFILE.dispatch_axis, DispatchAxis::ReturnType);
}

#[test]
fn haskell_async_wrappers_contain_io() {
    assert!(HASKELL_PROFILE.async_wrappers.contains(&"IO"));
}

#[test]
fn data_constructor_application_is_callable() {
    // `Str "x"` / `Div attrs xs`: a data constructor extracted as `EnumMember`,
    // applied as a `Calls` ref, must be a compatible target.
    assert!(KindCompatibility::check(
        HASKELL_KIND_TABLE,
        EdgeKind::Calls,
        SymbolKind::EnumMember,
    ));
}

#[test]
fn ordinary_function_call_unaffected() {
    assert!(KindCompatibility::check(
        HASKELL_KIND_TABLE,
        EdgeKind::Calls,
        SymbolKind::Function,
    ));
}

#[test]
fn data_constructor_is_not_a_type_target() {
    // The widening is `Calls`-only: a constructor is not a `TypeRef` target.
    assert!(!KindCompatibility::check(
        HASKELL_KIND_TABLE,
        EdgeKind::TypeRef,
        SymbolKind::EnumMember,
    ));
}
