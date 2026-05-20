use super::MAKO_PROFILE;

#[test]
fn mako_profile_identity_and_shadow_mode() {
    assert_eq!(MAKO_PROFILE.id, "mako");
    assert!(!MAKO_PROFILE.engine_primary);
}
