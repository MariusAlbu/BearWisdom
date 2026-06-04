use super::HASKELL_PROFILE;
use crate::type_checker::profile::language_profile::DispatchAxis;

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
