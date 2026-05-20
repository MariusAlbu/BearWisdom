use super::SMARTY_PROFILE;

#[test]
fn smarty_profile_identity_and_shadow_mode() {
    assert_eq!(SMARTY_PROFILE.id, "smarty");
    assert!(!SMARTY_PROFILE.engine_primary);
}
