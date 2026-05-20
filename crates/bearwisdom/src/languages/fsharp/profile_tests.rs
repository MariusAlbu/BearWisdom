use super::FSHARP_PROFILE;

#[test]
fn fsharp_profile_identity_and_shadow_mode() {
    assert_eq!(FSHARP_PROFILE.id, "fsharp");
    assert!(!FSHARP_PROFILE.engine_primary);
    assert!(FSHARP_PROFILE.async_wrappers.contains(&"Async"));
    assert!(FSHARP_PROFILE.async_wrappers.contains(&"Task"));
}
