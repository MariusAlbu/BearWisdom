use super::CMAKE_PROFILE;

#[test]
fn cmake_profile_identity_and_shadow_mode() {
    assert_eq!(CMAKE_PROFILE.id, "cmake");
    assert!(!CMAKE_PROFILE.engine_primary);
}
