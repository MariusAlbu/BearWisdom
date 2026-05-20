use super::HEEX_PROFILE;

#[test]
fn heex_profile_identity_and_shadow_mode() {
    assert_eq!(HEEX_PROFILE.id, "heex");
    assert!(!HEEX_PROFILE.engine_primary);
}
