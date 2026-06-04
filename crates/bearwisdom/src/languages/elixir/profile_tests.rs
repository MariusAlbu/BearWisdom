use super::ELIXIR_PROFILE;

#[test]
fn elixir_profile_identity() {
    assert_eq!(ELIXIR_PROFILE.id, "elixir");
    assert_eq!(ELIXIR_PROFILE.qname_separator, ".");
}
