use super::FORTRAN_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn fortran_profile_identity_and_shadow_mode() {
    assert_eq!(FORTRAN_PROFILE.id, "fortran");
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
