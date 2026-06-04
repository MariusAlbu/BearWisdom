use super::VBNET_PROFILE;

#[test]
fn vbnet_profile_identity_and_shadow_mode() {
    assert_eq!(VBNET_PROFILE.id, "vbnet");
    assert_eq!(VBNET_PROFILE.self_keywords, &["Me", "MyClass", "MyBase"]);
}
