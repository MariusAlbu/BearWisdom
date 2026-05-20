use super::FREEMARKER_PROFILE;

#[test]
fn freemarker_profile_identity_and_shadow_mode() {
    assert_eq!(FREEMARKER_PROFILE.id, "freemarker");
    assert!(!FREEMARKER_PROFILE.engine_primary);
}
