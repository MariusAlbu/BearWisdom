use super::GSP_PROFILE;

#[test]
fn gsp_profile_identity_and_shadow_mode() {
    assert_eq!(GSP_PROFILE.id, "gsp");
}
