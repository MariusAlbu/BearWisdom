use super::HANDLEBARS_PROFILE;

#[test]
fn handlebars_profile_identity_and_shadow_mode() {
    assert_eq!(HANDLEBARS_PROFILE.id, "handlebars");
}
