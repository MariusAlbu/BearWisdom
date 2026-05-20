use super::ANGULAR_PROFILE;

#[test]
fn angular_profile_identity_and_shadow_mode() {
    assert_eq!(ANGULAR_PROFILE.id, "angular");
    assert!(!ANGULAR_PROFILE.engine_primary);
}
