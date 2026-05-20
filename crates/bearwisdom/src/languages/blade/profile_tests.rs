use super::BLADE_PROFILE;

#[test]
fn blade_profile_identity_and_shadow_mode() {
    assert_eq!(BLADE_PROFILE.id, "blade");
    assert!(!BLADE_PROFILE.engine_primary);
}
