// =============================================================================
// matlab/profile_tests.rs — profile-axis and full-ladder bind tests.
//
// MATLAB rides the generic ladder with no resolve_ref hook. Two pure-data
// flags on MATLAB_PROFILE drive its bare-call binds:
//   * module_scope == SameDir — same-folder function sibling (MATLAB path
//     precedence) binds via the same-dir rung, which runs first.
//   * namespaceless_global_type_lookup — a cross-dir project function with no
//     same-dir sibling first-match-binds via the dead-last rung; toolbox
//     intrinsics (external) decline and stay external.
// =============================================================================

use super::MATLAB_PROFILE;

#[test]
fn matlab_profile_identity_and_shadow_mode() {
    assert_eq!(MATLAB_PROFILE.id, "matlab");
}

#[test]
fn matlab_module_scope_is_same_dir() {
    // MATLAB path semantics: a same-folder function sibling is the canonical
    // bind for a bare call. The same-dir rung is selected by SameDir.
    assert_eq!(
        MATLAB_PROFILE.imports.module_scope,
        crate::type_checker::profile::language_profile::ModuleScope::SameDir
    );
}

#[test]
fn matlab_namespaceless_global_is_on() {
    // A bare call to a cross-dir project function with no same-dir sibling
    // falls through to the dead-last first-match-by-name rung.
    assert_eq!(
        MATLAB_PROFILE.namespaceless_global_type_lookup,
        crate::type_checker::profile::language_profile::NamespaceScope::Global
    );
}

