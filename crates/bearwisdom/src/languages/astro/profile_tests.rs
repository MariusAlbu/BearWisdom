use super::ASTRO_PROFILE;

#[test]
fn astro_profile_identity_and_shadow_mode() {
    assert_eq!(ASTRO_PROFILE.id, "astro");
    assert!(!ASTRO_PROFILE.engine_primary);
}
