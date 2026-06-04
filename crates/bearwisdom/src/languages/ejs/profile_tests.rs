use super::EJS_PROFILE;

#[test]
fn ejs_profile_identity_and_shadow_mode() {
    assert_eq!(EJS_PROFILE.id, "ejs");
}
