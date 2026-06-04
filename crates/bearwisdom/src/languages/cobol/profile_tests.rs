use super::COBOL_PROFILE;

#[test]
fn cobol_profile_identity_and_shadow_mode() {
    assert_eq!(COBOL_PROFILE.id, "cobol");
}
