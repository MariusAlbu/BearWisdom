use super::RAZOR_PROFILE;

#[test]
fn razor_profile_identity_and_shadow_mode() {
    assert_eq!(RAZOR_PROFILE.id, "razor");
    assert!(!RAZOR_PROFILE.engine_primary);
}
