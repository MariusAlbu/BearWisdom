use super::MATLAB_PROFILE;

#[test]
fn matlab_profile_identity_and_shadow_mode() {
    assert_eq!(MATLAB_PROFILE.id, "matlab");
}
