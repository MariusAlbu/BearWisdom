use super::BASH_PROFILE;
use crate::type_checker::profile::language_profile::{KindCompatibility, NamespaceScope};
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn bash_profile_identity_and_shadow_mode() {
    assert_eq!(BASH_PROFILE.id, "shell");
}

#[test]
fn bash_arms_global_lookup_for_sourced_functions() {
    // A sourced shell function is project-global; the namespaceless-global rung
    // binds a bare call the source-path hook can't match.
    assert_eq!(
        BASH_PROFILE.namespaceless_global_type_lookup,
        NamespaceScope::Global
    );
}

#[test]
fn bash_kind_table_matches_former_predicate() {
    let t = BASH_PROFILE.kind_compatible_table;
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
    assert!(KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Variable
    ));
    assert!(BASH_PROFILE.builtin_skip.is_some());
}
