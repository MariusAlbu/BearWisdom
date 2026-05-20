use super::ODIN_PROFILE;

#[test]
fn odin_profile_identity_and_shadow_mode() {
    assert_eq!(ODIN_PROFILE.id, "odin");
    assert!(!ODIN_PROFILE.engine_primary);
}
