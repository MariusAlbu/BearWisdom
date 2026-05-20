use super::GLEAM_PROFILE;

#[test]
fn gleam_profile_identity_and_shadow_mode() {
    assert_eq!(GLEAM_PROFILE.id, "gleam");
    assert!(!GLEAM_PROFILE.engine_primary);
}
