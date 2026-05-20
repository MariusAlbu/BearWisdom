use super::PROLOG_PROFILE;

#[test]
fn prolog_profile_identity_and_shadow_mode() {
    assert_eq!(PROLOG_PROFILE.id, "prolog");
    assert!(!PROLOG_PROFILE.engine_primary);
}
