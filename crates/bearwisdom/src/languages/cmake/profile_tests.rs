// =============================================================================
// cmake/profile_tests.rs — profile-axis, kind-table, and full-ladder bind tests.
//
// CMake variables and functions/macros are project-global: they flow down
// `add_subdirectory`, so a bare `${VAR}` or command ref binds across the build
// tree. `namespaceless_global_type_lookup == Global` drives that bind via the
// dead-last first-match-by-name rung; a same-named ext: toolchain stub declines
// and stays external.
// =============================================================================

use super::CMAKE_PROFILE;
use crate::type_checker::profile::language_profile::{KindCompatibility, NamespaceScope};
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn cmake_profile_identity_and_shadow_mode() {
    assert_eq!(CMAKE_PROFILE.id, "cmake");
}

#[test]
fn cmake_namespaceless_global_is_on() {
    // CMake variables/functions are build-tree global, so a bare ref binds via
    // the dead-last first-match-by-name rung.
    assert_eq!(
        CMAKE_PROFILE.namespaceless_global_type_lookup,
        NamespaceScope::Global
    );
}

#[test]
fn cmake_kind_table_matches_former_predicate() {
    let t = CMAKE_PROFILE.kind_compatible_table;
    // Calls → function (macros extract as Function).
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Function
    ));
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Variable
    ));
    // TypeRef → variable | function.
    assert!(KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Variable
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Function
    ));
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Class
    ));
    // Unlisted edge kinds stay permissive.
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Imports,
        SymbolKind::Namespace
    ));
}

