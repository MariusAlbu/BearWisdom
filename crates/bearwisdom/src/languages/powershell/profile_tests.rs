use super::POWERSHELL_PROFILE;

#[test]
fn powershell_profile_identity_and_shadow_mode() {
    assert_eq!(POWERSHELL_PROFILE.id, "powershell");
    assert_eq!(POWERSHELL_PROFILE.self_keywords, &["$this"]);
}
