use super::PASCAL_PROFILE;
use crate::type_checker::profile::language_profile::{NameNormalization, WildcardMatch};

#[test]
fn pascal_profile_identity_and_shadow_mode() {
    assert_eq!(PASCAL_PROFILE.id, "pascal");
    assert_eq!(PASCAL_PROFILE.self_keywords, &["Self"]);
}

#[test]
fn pascal_case_folds_and_matches_units_by_file_stem() {
    // The former resolve_ref case-insensitive same-file match drains to the
    // case-folding NameNormalization; the wildcard unit-import match drains to
    // FileStem with the `{unit}_…` include-file probe.
    assert!(matches!(
        PASCAL_PROFILE.name_normalization,
        NameNormalization::Spec(spec) if spec.case_insensitive
    ));
    assert!(matches!(
        PASCAL_PROFILE.wildcard_match,
        WildcardMatch::FileStem { underscore_prefix: true }
    ));
}
