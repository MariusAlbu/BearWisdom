use super::NIM_PROFILE;

#[test]
fn nim_profile_identity_and_shadow_mode() {
    assert_eq!(NIM_PROFILE.id, "nim");
    assert!(NIM_PROFILE.has_generics);
}
