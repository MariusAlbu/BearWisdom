use super::GRAPHQL_PROFILE;

#[test]
fn graphql_profile_identity_and_shadow_mode() {
    assert_eq!(GRAPHQL_PROFILE.id, "graphql");
    assert!(!GRAPHQL_PROFILE.engine_primary);
}
