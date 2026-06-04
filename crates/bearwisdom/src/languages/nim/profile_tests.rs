use super::NIM_PROFILE;
use crate::type_checker::profile::language_profile::ExtMatch;

#[test]
fn nim_profile_identity_and_shadow_mode() {
    assert_eq!(NIM_PROFILE.id, "nim");
    assert!(NIM_PROFILE.has_generics);
}

#[test]
fn nim_binds_imports_to_file_named_externals() {
    // The former resolve_ref import-leaf / package external tiers drain to the
    // import-scoped external bind matched by file-stem / dir against imports.
    // The unconditional any-stdlib guess tier is intentionally NOT reproduced.
    assert!(NIM_PROFILE.external_by_import.is_some());
    assert_eq!(NIM_PROFILE.ext_match, ExtMatch::FileStemOrDir);
}
