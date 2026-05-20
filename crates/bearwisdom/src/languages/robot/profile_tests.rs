use super::ROBOT_PROFILE;

#[test]
fn robot_profile_identity_and_shadow_mode() {
    assert_eq!(ROBOT_PROFILE.id, "robot");
    assert!(!ROBOT_PROFILE.engine_primary);
}
