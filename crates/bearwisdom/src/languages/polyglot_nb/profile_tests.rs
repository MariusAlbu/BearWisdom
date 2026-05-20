use super::POLYGLOT_NB_PROFILE;

#[test]
fn polyglot_nb_profile_identity_and_shadow_mode() {
    assert_eq!(POLYGLOT_NB_PROFILE.id, "polyglot_nb");
    assert!(!POLYGLOT_NB_PROFILE.engine_primary);
}
