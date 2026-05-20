use super::VELOCITY_PROFILE;

#[test]
fn velocity_profile_identity_and_shadow_mode() {
    assert_eq!(VELOCITY_PROFILE.id, "velocity");
    assert!(!VELOCITY_PROFILE.engine_primary);
}
