use super::VBA_PROFILE;

#[test]
fn vba_profile_identity_and_shadow_mode() {
    assert_eq!(VBA_PROFILE.id, "vba");
    assert_eq!(VBA_PROFILE.self_keywords, &["Me"]);
    assert!(!VBA_PROFILE.engine_primary);
}
