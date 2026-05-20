use super::SLIM_PROFILE;

#[test]
fn slim_profile_identity_and_shadow_mode() {
    assert_eq!(SLIM_PROFILE.id, "slim");
    assert!(!SLIM_PROFILE.engine_primary);
}
