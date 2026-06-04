use super::HARE_PROFILE;
use crate::type_checker::profile::language_profile::ChainQualification;

#[test]
fn hare_profile_identity_and_shadow_mode() {
    assert_eq!(HARE_PROFILE.id, "hare");
}

#[test]
fn hare_profile_drains_resolve_ref_to_engine_data() {
    // builtin_skip declines Hare primitives before the ladder; PackageShortName
    // binds a bare `mod::target` member through the generic engine.
    assert!(HARE_PROFILE.builtin_skip.is_some());
    assert_eq!(HARE_PROFILE.qname_separator, "::");
    assert_eq!(
        HARE_PROFILE.chain_qualification,
        ChainQualification::PackageShortName
    );
    let is_builtin = HARE_PROFILE.builtin_skip.unwrap();
    assert!(is_builtin("int"));
    assert!(is_builtin("str"));
    assert!(!is_builtin("printfln"));
}
