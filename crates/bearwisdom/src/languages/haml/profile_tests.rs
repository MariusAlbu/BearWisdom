use super::HAML_PROFILE;

#[test]
fn haml_profile_identity_and_shadow_mode() {
    assert_eq!(HAML_PROFILE.id, "haml");
    assert!(!HAML_PROFILE.engine_primary);
}
