use super::DOCKERFILE_PROFILE;

#[test]
fn dockerfile_profile_identity_and_shadow_mode() {
    assert_eq!(DOCKERFILE_PROFILE.id, "dockerfile");
    assert!(!DOCKERFILE_PROFILE.engine_primary);
}
