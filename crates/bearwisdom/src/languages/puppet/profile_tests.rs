use super::PUPPET_PROFILE;

#[test]
fn puppet_profile_identity_and_shadow_mode() {
    assert_eq!(PUPPET_PROFILE.id, "puppet");
    assert_eq!(PUPPET_PROFILE.qname_separator, "::");
}

#[test]
fn puppet_declines_qualified_target_under_imported_module() {
    // A `module::pred` target whose leading `::`-segment names a declared
    // dependency module is external; the engine's import-prefix decline drains
    // the former resolve_ref import-set decline branch.
    assert!(PUPPET_PROFILE.decline_qualified_when_prefix_imported);
}
