use super::ERLANG_PROFILE;

#[test]
fn erlang_profile_identity() {
    assert_eq!(ERLANG_PROFILE.id, "erlang");
    assert_eq!(ERLANG_PROFILE.qname_separator, ":");
}

#[test]
fn erlang_profile_engine_primary_disabled() {
    assert!(!ERLANG_PROFILE.engine_primary);
}
