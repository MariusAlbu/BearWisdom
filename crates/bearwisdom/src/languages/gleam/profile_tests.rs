use super::GLEAM_PROFILE;
use crate::type_checker::profile::language_profile::ChainQualification;

#[test]
fn gleam_profile_identity_and_shadow_mode() {
    assert_eq!(GLEAM_PROFILE.id, "gleam");
}

#[test]
fn gleam_uses_package_short_name_qualification() {
    // `import gleam/io` brings short name `io`; bare members resolve under
    // `io.{target}` via the engine's package-short-name strategy.
    assert_eq!(
        GLEAM_PROFILE.chain_qualification,
        ChainQualification::PackageShortName
    );
    assert!(GLEAM_PROFILE.builtin_skip.is_some());
}
