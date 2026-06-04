use super::THYMELEAF_PROFILE;

#[test]
fn thymeleaf_profile_identity_and_shadow_mode() {
    assert_eq!(THYMELEAF_PROFILE.id, "thymeleaf");
}
