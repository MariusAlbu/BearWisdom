use super::BASH_PROFILE;

#[test]
fn bash_profile_identity_and_shadow_mode() {
    assert_eq!(BASH_PROFILE.id, "shell");
}
