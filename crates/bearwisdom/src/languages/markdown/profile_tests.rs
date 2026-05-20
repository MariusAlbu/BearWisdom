use super::MARKDOWN_PROFILE;

#[test]
fn markdown_profile_identity_and_shadow_mode() {
    assert_eq!(MARKDOWN_PROFILE.id, "markdown");
    assert!(!MARKDOWN_PROFILE.engine_primary);
}
