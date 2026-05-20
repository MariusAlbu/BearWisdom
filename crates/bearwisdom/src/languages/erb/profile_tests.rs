use super::ERB_PROFILE;

#[test]
fn erb_profile_identity_and_shadow_mode() {
    assert_eq!(ERB_PROFILE.id, "erb");
    assert!(!ERB_PROFILE.engine_primary);
}
