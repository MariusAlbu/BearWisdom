use super::ZIG_PROFILE;

#[test]
fn zig_profile_identity_and_shadow_mode() {
    assert_eq!(ZIG_PROFILE.id, "zig");
    assert!(!ZIG_PROFILE.engine_primary);
}
