use super::NIX_PROFILE;

#[test]
fn nix_profile_identity_and_shadow_mode() {
    assert_eq!(NIX_PROFILE.id, "nix");
    assert!(!NIX_PROFILE.engine_primary);
}
