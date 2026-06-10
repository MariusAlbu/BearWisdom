use super::C_LANG_PROFILE;

#[test]
fn c_profile_identity_and_shadow_mode() {
    assert_eq!(C_LANG_PROFILE.id, "c");
}

#[test]
fn c_profile_namespace_decline_gates_r_c_api() {
    // The R-package C-API decline is namespace-gated profile data: armed by the
    // R-package file namespace, reserves the R C API symbol set.
    let nd = C_LANG_PROFILE
        .namespace_decline
        .expect("c profile declares a namespace decline");
    assert_eq!(
        nd.file_namespace,
        crate::languages::c_lang::hooks::R_PACKAGE_SENTINEL
    );
    assert!((nd.is_reserved)("Rf_eval"));
    assert!(!(nd.is_reserved)("my_project_fn"));
}
