use super::HCL_PROFILE;

#[test]
fn hcl_profile_identity_and_shadow_mode() {
    assert_eq!(HCL_PROFILE.id, "hcl");
}
