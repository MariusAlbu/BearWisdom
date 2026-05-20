use super::CRONTAB_PROFILE;

#[test]
fn crontab_profile_identity_and_shadow_mode() {
    assert_eq!(CRONTAB_PROFILE.id, "crontab");
    assert!(!CRONTAB_PROFILE.engine_primary);
}
