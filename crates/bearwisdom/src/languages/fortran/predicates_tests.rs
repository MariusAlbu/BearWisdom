// =============================================================================
// fortran/predicates_tests.rs — intrinsic-procedure/module drain predicate
// =============================================================================

use super::is_fortran_intrinsic;

#[test]
fn recognizes_intrinsic_procedures_across_categories() {
    for name in ["abs", "size", "real", "allocated", "max", "present", "reshape", "min", "mod"] {
        assert!(is_fortran_intrinsic(name), "expected '{name}' to be intrinsic");
    }
}

#[test]
fn recognizes_ieee_and_iso_c_binding_procedures() {
    assert!(is_fortran_intrinsic("ieee_is_nan"));
    assert!(is_fortran_intrinsic("ieee_value"));
    assert!(is_fortran_intrinsic("c_associated"));
    assert!(is_fortran_intrinsic("c_f_pointer"));
}

#[test]
fn recognizes_intrinsic_modules() {
    assert!(is_fortran_intrinsic("iso_c_binding"));
    assert!(is_fortran_intrinsic("ieee_arithmetic"));
    assert!(is_fortran_intrinsic("ieee_exceptions"));
    assert!(is_fortran_intrinsic("ieee_features"));
}

/// Fortran is case-insensitive; source may spell an intrinsic in any case.
#[test]
fn matches_case_insensitively() {
    assert!(is_fortran_intrinsic("SIZE"));
    assert!(is_fortran_intrinsic("Real"));
    assert!(is_fortran_intrinsic("ALLOCATED"));
    assert!(is_fortran_intrinsic("Ieee_Arithmetic"));
}

/// Names that stdlib-shaped Fortran projects commonly re-declare as a
/// same-named generic interface must NOT be drained — the ladder needs to
/// reach the project's own definition instead of being preempted here.
#[test]
fn declines_names_commonly_shadowed_by_project_generic_interfaces() {
    for name in [
        "adjustl", "adjustr", "char", "count", "iachar", "ichar", "index", "len", "len_trim",
        "lge", "lgt", "lle", "llt", "merge", "random_seed", "repeat", "scan", "transpose",
        "trim", "unpack", "verify",
    ] {
        assert!(!is_fortran_intrinsic(name), "expected '{name}' NOT to be drained");
    }
}

/// A project may legally declare its own module named `iso_fortran_env`
/// (observed in a test fixture) — it must reach the ladder, not be drained.
#[test]
fn declines_iso_fortran_env_module_name() {
    assert!(!is_fortran_intrinsic("iso_fortran_env"));
}

#[test]
fn declines_ordinary_project_names() {
    assert!(!is_fortran_intrinsic("compute_stress"));
    assert!(!is_fortran_intrinsic("MyDerivedType"));
}
