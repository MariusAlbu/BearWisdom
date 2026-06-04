use super::CMAKE_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn cmake_profile_identity_and_shadow_mode() {
    assert_eq!(CMAKE_PROFILE.id, "cmake");
}

#[test]
fn cmake_kind_table_matches_former_predicate() {
    let t = CMAKE_PROFILE.kind_compatible_table;
    // Calls → function (macros extract as Function).
    assert!(KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::Function));
    assert!(!KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::Variable));
    // TypeRef → variable | function.
    assert!(KindCompatibility::check(t, EdgeKind::TypeRef, SymbolKind::Variable));
    assert!(KindCompatibility::check(t, EdgeKind::TypeRef, SymbolKind::Function));
    assert!(!KindCompatibility::check(t, EdgeKind::TypeRef, SymbolKind::Class));
    // Unlisted edge kinds stay permissive.
    assert!(KindCompatibility::check(t, EdgeKind::Imports, SymbolKind::Namespace));
}
