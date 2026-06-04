use super::SYSTEMD_PROFILE;

#[test]
fn systemd_profile_identity_and_shadow_mode() {
    assert_eq!(SYSTEMD_PROFILE.id, "systemd");
}
