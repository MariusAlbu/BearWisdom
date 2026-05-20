use super::PUPPET_PROFILE;

#[test]
fn puppet_profile_identity_and_shadow_mode() {
    assert_eq!(PUPPET_PROFILE.id, "puppet");
    assert_eq!(PUPPET_PROFILE.qname_separator, "::");
    assert!(!PUPPET_PROFILE.engine_primary);
}
