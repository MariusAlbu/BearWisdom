use super::R_PROFILE;
use crate::type_checker::profile::language_profile::DispatchAxis;

#[test]
fn r_profile_identity_and_shadow_mode() {
    assert_eq!(R_PROFILE.id, "r");
    assert_eq!(R_PROFILE.qname_separator, "::");
    assert!(!R_PROFILE.engine_primary);
}

#[test]
fn r_dispatch_axis_is_multi_arg_for_s4() {
    assert_eq!(R_PROFILE.dispatch_axis, DispatchAxis::MultiArg);
}
