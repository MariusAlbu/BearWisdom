use super::JAVASCRIPT_PROFILE;

#[test]
fn javascript_profile_identity_and_shadow_mode() {
    assert_eq!(JAVASCRIPT_PROFILE.id, "javascript");
    assert_eq!(JAVASCRIPT_PROFILE.self_keywords, &["this"]);
    assert!(!JAVASCRIPT_PROFILE.engine_primary);
    assert!(JAVASCRIPT_PROFILE.async_wrappers.contains(&"Promise"));
}
