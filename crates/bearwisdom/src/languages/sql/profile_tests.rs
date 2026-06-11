use super::SQL_PROFILE;
use crate::type_checker::profile::language_profile::NameNormalization;

#[test]
fn sql_profile_identity_and_shadow_mode() {
    assert_eq!(SQL_PROFILE.id, "sql");
}

#[test]
fn sql_name_normalization_is_case_insensitive() {
    match SQL_PROFILE.name_normalization {
        NameNormalization::Spec(spec) => {
            assert!(spec.case_insensitive, "SQL is case-insensitive per ANSI");
            assert!(spec.strip_chars.is_empty());
            assert!(spec.strip_prefixes.is_empty());
            assert!(spec.strip_sigils.is_empty());
        }
        NameNormalization::None => panic!("SQL must fold case in name comparison"),
    }
}
