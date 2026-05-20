use super::FORTRAN_PROFILE;

#[test]
fn fortran_profile_identity_and_shadow_mode() {
    assert_eq!(FORTRAN_PROFILE.id, "fortran");
    assert!(!FORTRAN_PROFILE.engine_primary);
}
