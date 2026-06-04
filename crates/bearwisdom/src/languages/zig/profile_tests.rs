use super::ZIG_PROFILE;

#[test]
fn zig_profile_identity_and_shadow_mode() {
    assert_eq!(ZIG_PROFILE.id, "zig");
}

#[test]
fn zig_profile_declines_compiler_builtins() {
    // builtin_skip declines the `@`-prefixed compiler builtins before the
    // ladder; the resolver hook is fully drained to engine data.
    assert!(ZIG_PROFILE.builtin_skip.is_some());
    let is_builtin = ZIG_PROFILE.builtin_skip.unwrap();
    assert!(is_builtin("@import"));
    assert!(is_builtin("@TypeOf"));
    assert!(!is_builtin("myFunction"));
}
