use super::GOTEMPLATE_PROFILE;

#[test]
fn gotemplate_profile_identity_and_shadow_mode() {
    assert_eq!(GOTEMPLATE_PROFILE.id, "gotemplate");
    assert!(!GOTEMPLATE_PROFILE.engine_primary);
}
