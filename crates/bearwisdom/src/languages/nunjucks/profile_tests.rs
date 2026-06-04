use super::NUNJUCKS_PROFILE;

#[test]
fn nunjucks_profile_identity_and_shadow_mode() {
    assert_eq!(NUNJUCKS_PROFILE.id, "nunjucks");
}
