use super::TEMPL_PROFILE;

#[test]
fn templ_profile_identity_and_shadow_mode() {
    assert_eq!(TEMPL_PROFILE.id, "templ");
    assert!(!TEMPL_PROFILE.engine_primary);
}
