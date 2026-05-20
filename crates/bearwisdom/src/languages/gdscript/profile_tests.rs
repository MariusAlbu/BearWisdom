use super::GDSCRIPT_PROFILE;

#[test]
fn gdscript_profile_identity_and_shadow_mode() {
    assert_eq!(GDSCRIPT_PROFILE.id, "gdscript");
    assert!(!GDSCRIPT_PROFILE.engine_primary);
}
