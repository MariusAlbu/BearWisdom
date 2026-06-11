use super::VBA_PROFILE;
use crate::type_checker::profile::language_profile::NamespaceScope;

#[test]
fn vba_profile_identity_and_shadow_mode() {
    assert_eq!(VBA_PROFILE.id, "vba");
    assert_eq!(VBA_PROFILE.self_keywords, &["Me"]);
}

#[test]
fn vba_binds_bare_refs_flat_globally() {
    // No import mechanism — bare references resolve flat-globally.
    assert_eq!(
        VBA_PROFILE.namespaceless_global_type_lookup,
        NamespaceScope::Global
    );
}
