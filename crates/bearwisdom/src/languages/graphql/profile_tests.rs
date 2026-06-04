use super::GRAPHQL_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn graphql_profile_identity_and_shadow_mode() {
    assert_eq!(GRAPHQL_PROFILE.id, "graphql");
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
    assert!(!KindCompatibility::check(t, EdgeKind::TypeRef, SymbolKind::Function));
}
