use super::SQL_PROFILE;

#[test]
fn sql_profile_identity_and_shadow_mode() {
    assert_eq!(SQL_PROFILE.id, "sql");
    assert!(!SQL_PROFILE.engine_primary);
}
