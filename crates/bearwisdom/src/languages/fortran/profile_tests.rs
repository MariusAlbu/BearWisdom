use super::FORTRAN_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn fortran_profile_identity_and_shadow_mode() {
    assert_eq!(FORTRAN_PROFILE.id, "fortran");
}

#[test]
fn fortran_builtin_skip_declines_intrinsics_not_project_names() {
    // Compiler intrinsics decline (classified builtin, not counted as an
    // unresolved project ref); an ordinary project-declared name is NOT in
    // the skip set and reaches the resolution ladder.
    let skip = FORTRAN_PROFILE.builtin_skip.expect("fortran builtin_skip set");
    assert!(skip("size"));
    assert!(skip("SIZE"));
    assert!(skip("allocated"));
    assert!(!skip("compute_stress"));
    // Names commonly re-declared as a project generic interface must reach
    // the ladder, not be preempted by the drain.
    assert!(!skip("merge"));
}

#[test]
fn fortran_inherits_row_targets_derived_type_struct() {
    let t = FORTRAN_PROFILE.kind_compatible_table;
    // EXTENDS(base) produces an Inherits edge to the base derived type, which
    // the extractor emits as a Struct.
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Inherits,
        SymbolKind::Struct
    ));
    // A non-type kind is rejected for Inherits.
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::Inherits,
        SymbolKind::Function
    ));
}
