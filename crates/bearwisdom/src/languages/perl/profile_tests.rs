use super::PERL_PROFILE;

#[test]
fn perl_profile_identity_and_shadow_mode() {
    assert_eq!(PERL_PROFILE.id, "perl");
    assert_eq!(PERL_PROFILE.qname_separator, "::");
    assert_eq!(PERL_PROFILE.self_keywords, &["$self"]);
    assert!(!PERL_PROFILE.engine_primary);
}
