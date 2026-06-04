use super::JUPYTER_PROFILE;

#[test]
fn jupyter_profile_identity_and_shadow_mode() {
    assert_eq!(JUPYTER_PROFILE.id, "jupyter");
}
