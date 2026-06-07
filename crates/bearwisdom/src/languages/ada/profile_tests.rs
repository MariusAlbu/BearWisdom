use super::ADA_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn ada_profile_identity_and_shadow_mode() {
    assert_eq!(ADA_PROFILE.id, "ada");
}

#[test]
fn ada_calls_row_accepts_type_conversions_and_functions() {
    // With the resolution hook deleted, the generic resolver consults this
    // table. Ada's parens-everywhere syntax means a Calls ref may bind to a
    // type (conversion `UInt16(x)`) as well as a function/procedure.
    let t = ADA_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::Struct));
    assert!(KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::TypeAlias));
    assert!(KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::Function));
}

#[test]
fn ada_builtin_skip_declines_runtime_names_not_project_types() {
    // Standard/Interfaces runtime names decline (classified builtin); the
    // project's own HAL types (UInt16 et al.) are NOT in the skip set so they
    // resolve through the ladder.
    let skip = ADA_PROFILE.builtin_skip.expect("ada builtin_skip set");
    assert!(skip("Long_Integer"));
    assert!(skip("Shift_Left"));
    assert!(!skip("UInt16"));
}
