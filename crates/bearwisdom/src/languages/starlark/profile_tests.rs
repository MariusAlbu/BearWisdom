use super::STARLARK_PROFILE;

#[test]
fn starlark_profile_identity_and_shadow_mode() {
    assert_eq!(STARLARK_PROFILE.id, "starlark");
    assert!(!STARLARK_PROFILE.engine_primary);
}
