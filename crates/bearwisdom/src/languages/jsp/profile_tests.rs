use super::JSP_PROFILE;

#[test]
fn jsp_profile_identity_and_shadow_mode() {
    assert_eq!(JSP_PROFILE.id, "jsp");
    assert!(!JSP_PROFILE.engine_primary);
}
