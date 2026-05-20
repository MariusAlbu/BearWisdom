use super::C_LANG_PROFILE;

#[test]
fn c_profile_identity_and_shadow_mode() {
    assert_eq!(C_LANG_PROFILE.id, "c");
    assert!(!C_LANG_PROFILE.engine_primary);
}
