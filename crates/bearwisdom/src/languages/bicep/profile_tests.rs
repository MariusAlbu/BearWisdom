use super::BICEP_PROFILE;

#[test]
fn bicep_profile_identity_and_shadow_mode() {
    assert_eq!(BICEP_PROFILE.id, "bicep");
    assert!(!BICEP_PROFILE.engine_primary);
}
