use super::LIQUID_PROFILE;

#[test]
fn liquid_profile_identity_and_shadow_mode() {
    assert_eq!(LIQUID_PROFILE.id, "liquid");
    assert!(!LIQUID_PROFILE.engine_primary);
}
