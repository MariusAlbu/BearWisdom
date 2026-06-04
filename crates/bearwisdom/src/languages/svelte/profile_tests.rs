use super::SVELTE_PROFILE;

#[test]
fn svelte_profile_identity_and_shadow_mode() {
    assert_eq!(SVELTE_PROFILE.id, "svelte");
}
