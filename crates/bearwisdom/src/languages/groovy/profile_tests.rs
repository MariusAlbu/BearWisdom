use super::GROOVY_PROFILE;

#[test]
fn groovy_profile_identity_and_shadow_mode() {
    assert_eq!(GROOVY_PROFILE.id, "groovy");
    assert_eq!(GROOVY_PROFILE.self_keywords, &["this", "super"]);
    assert!(!GROOVY_PROFILE.engine_primary);
}
