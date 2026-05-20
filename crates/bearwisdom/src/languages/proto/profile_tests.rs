use super::PROTO_PROFILE;

#[test]
fn proto_profile_identity_and_shadow_mode() {
    assert_eq!(PROTO_PROFILE.id, "proto");
    assert!(!PROTO_PROFILE.engine_primary);
}
