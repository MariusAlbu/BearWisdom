use super::JINJA_PROFILE;

#[test]
fn jinja_profile_identity_and_shadow_mode() {
    assert_eq!(JINJA_PROFILE.id, "jinja");
}
