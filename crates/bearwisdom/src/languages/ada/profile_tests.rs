use super::ADA_PROFILE;

#[test]
fn ada_profile_identity_and_shadow_mode() {
    assert_eq!(ADA_PROFILE.id, "ada");
    assert!(!ADA_PROFILE.engine_primary);
}
