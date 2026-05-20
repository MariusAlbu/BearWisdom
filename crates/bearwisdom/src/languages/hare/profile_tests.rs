use super::HARE_PROFILE;

#[test]
fn hare_profile_identity_and_shadow_mode() {
    assert_eq!(HARE_PROFILE.id, "hare");
    assert!(!HARE_PROFILE.engine_primary);
}
