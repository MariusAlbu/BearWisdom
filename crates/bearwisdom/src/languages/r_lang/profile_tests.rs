// =============================================================================
// r_lang/profile_tests.rs — profile-axis and full-ladder bind tests.
//
// R is a flat top-level function namespace: a project's own exported functions
// (defined in sibling .R files) are called bare with no scope/import structure.
// `namespaceless_global_type_lookup` binds such a bare call first-match to the
// project-internal definition; a same-named external stub (testthat/base) loses
// because the rung excludes external files. Qualified `pkg::fn` calls carry the
// `::` separator and stay external (the bare-name rung never sees them).
// =============================================================================

use super::R_PROFILE;
use crate::type_checker::profile::language_profile::DispatchAxis;

#[test]
fn r_profile_identity_and_shadow_mode() {
    assert_eq!(R_PROFILE.id, "r");
    assert_eq!(R_PROFILE.qname_separator, "::");
}

#[test]
fn r_dispatch_axis_is_multi_arg_for_s4() {
    assert_eq!(R_PROFILE.dispatch_axis, DispatchAxis::MultiArg);
}

#[test]
fn r_namespaceless_global_is_on() {
    // R's flat function namespace binds bare calls to the project's own
    // exported functions via the dead-last first-match-by-name rung.
    assert_eq!(
        R_PROFILE.namespaceless_global_type_lookup,
        crate::type_checker::profile::language_profile::NamespaceScope::Global
    );
}

