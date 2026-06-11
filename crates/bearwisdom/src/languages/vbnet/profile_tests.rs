use super::VBNET_PROFILE;
use crate::type_checker::profile::language_profile::NameNormalization;

#[test]
fn vbnet_profile_identity_and_shadow_mode() {
    assert_eq!(VBNET_PROFILE.id, "vbnet");
    assert_eq!(VBNET_PROFILE.self_keywords, &["Me", "MyClass", "MyBase"]);
}

#[test]
fn vbnet_name_normalization_is_case_insensitive() {
    match VBNET_PROFILE.name_normalization {
        NameNormalization::Spec(spec) => {
            assert!(spec.case_insensitive, "VB.NET is case-insensitive by spec");
            // Folding only — no sigil/prefix/char stripping.
            assert!(spec.strip_chars.is_empty());
            assert!(spec.strip_prefixes.is_empty());
            assert!(spec.strip_sigils.is_empty());
        }
        NameNormalization::None => panic!("VB.NET must fold case in name comparison"),
    }
}
