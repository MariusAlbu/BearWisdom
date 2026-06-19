// =============================================================================
// graphql/profile_tests.rs — profile-axis, kind-table, and full-ladder binds.
//
// GraphQL SDL types share one flat schema namespace across `.graphql` files, so
// a bare type / custom-scalar ref binds to its sibling-file declaration.
// `namespaceless_global_type_lookup == Global` drives that bind via the
// dead-last first-match-by-name rung; a same-named ext: schema stub declines
// and stays external.
// =============================================================================

use super::GRAPHQL_PROFILE;
use crate::type_checker::profile::language_profile::{KindCompatibility, NamespaceScope};
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn graphql_profile_identity_and_shadow_mode() {
    assert_eq!(GRAPHQL_PROFILE.id, "graphql");
}

#[test]
fn graphql_namespaceless_global_is_on() {
    // SDL types are schema-flat across `.graphql` files, so a bare type ref
    // binds via the dead-last first-match-by-name rung.
    assert_eq!(
        GRAPHQL_PROFILE.namespaceless_global_type_lookup,
        NamespaceScope::Global
    );
}

#[test]
fn graphql_type_ref_accepts_type_kinds_rejects_callables() {
    let t = GRAPHQL_PROFILE.kind_compatible_table;
    for k in [
        SymbolKind::Class,
        SymbolKind::Interface,
        SymbolKind::Enum,
        SymbolKind::Struct,
        SymbolKind::TypeAlias,
    ] {
        assert!(KindCompatibility::check(t, EdgeKind::TypeRef, k));
    }
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Function
    ));
}

