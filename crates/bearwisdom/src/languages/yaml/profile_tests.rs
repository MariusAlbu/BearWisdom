use super::YAML_PROFILE;

#[test]
fn yaml_profile_identity_and_shadow_mode() {
    assert_eq!(YAML_PROFILE.id, "yaml");
    assert!(!YAML_PROFILE.engine_primary);
}
