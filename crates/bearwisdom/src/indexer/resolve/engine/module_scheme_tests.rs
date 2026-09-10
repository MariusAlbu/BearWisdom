use super::strip_scheme_prefix;
use crate::type_checker::profile::language_profile::{LanguageProfile, DEFAULT_PROFILE};

#[test]
fn uri_scheme_strips() {
    assert_eq!(
        strip_scheme_prefix(&DEFAULT_PROFILE, "custom:assert/strict"),
        Some("assert/strict")
    );
    assert_eq!(
        strip_scheme_prefix(&DEFAULT_PROFILE, "virtual:module"),
        Some("module")
    );
    assert_eq!(
        strip_scheme_prefix(&DEFAULT_PROFILE, "jsr:@std/path"),
        Some("@std/path")
    );
}
#[test]
fn qualified_paths_are_not_schemes() {
    assert_eq!(
        strip_scheme_prefix(&crate::languages::rust_lang::RUST_PROFILE, "crate::db"),
        None
    );
    assert_eq!(
        strip_scheme_prefix(
            &crate::languages::rust_lang::RUST_PROFILE,
            "std::collections::HashMap"
        ),
        None
    );
}

#[test]
fn active_profile_owns_qualified_separator_syntax() {
    let profile = LanguageProfile {
        qname_separator: ":",
        ..DEFAULT_PROFILE
    };
    assert_eq!(strip_scheme_prefix(&profile, "namespace:member"), None);
}

#[test]
fn non_scheme_shapes_pass_through() {
    assert_eq!(strip_scheme_prefix(&DEFAULT_PROFILE, "assert/strict"), None);
    assert_eq!(strip_scheme_prefix(&DEFAULT_PROFILE, "./relative"), None);
    assert_eq!(strip_scheme_prefix(&DEFAULT_PROFILE, ":leading"), None);
    assert_eq!(strip_scheme_prefix(&DEFAULT_PROFILE, "custom:"), None);
    assert_eq!(strip_scheme_prefix(&DEFAULT_PROFILE, "1um:x"), None);
}
