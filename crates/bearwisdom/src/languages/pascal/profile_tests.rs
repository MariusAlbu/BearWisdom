use super::PASCAL_PROFILE;

#[test]
fn pascal_profile_identity_and_shadow_mode() {
    assert_eq!(PASCAL_PROFILE.id, "pascal");
    assert_eq!(PASCAL_PROFILE.self_keywords, &["Self"]);
}
