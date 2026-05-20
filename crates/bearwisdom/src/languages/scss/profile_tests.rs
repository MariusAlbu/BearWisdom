use super::SCSS_PROFILE;

#[test]
fn scss_profile_identity_and_shadow_mode() {
    assert_eq!(SCSS_PROFILE.id, "scss");
    assert!(!SCSS_PROFILE.engine_primary);
}
