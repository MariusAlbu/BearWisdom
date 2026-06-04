use super::RUST_PROFILE;
use crate::type_checker::profile::language_profile::{KindCompatibility, SupertypeDiscovery};
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn rust_profile_identity() {
    assert_eq!(RUST_PROFILE.id, "rust");
    assert_eq!(RUST_PROFILE.qname_separator, ".");
    assert_eq!(RUST_PROFILE.self_keywords, &["self", "Self"]);
}

#[test]
fn rust_calls_accepts_function_method_constructor_closure_bindings() {
    let table = RUST_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(table, EdgeKind::Calls, SymbolKind::Function));
    assert!(KindCompatibility::check(table, EdgeKind::Calls, SymbolKind::Method));
    assert!(KindCompatibility::check(table, EdgeKind::Calls, SymbolKind::Constructor));
    assert!(KindCompatibility::check(table, EdgeKind::Calls, SymbolKind::Variable));
    assert!(KindCompatibility::check(table, EdgeKind::Calls, SymbolKind::Parameter));
}

#[test]
fn rust_inherits_accepts_trait_only() {
    let table = RUST_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(table, EdgeKind::Inherits, SymbolKind::Trait));
    assert!(!KindCompatibility::check(table, EdgeKind::Inherits, SymbolKind::Struct));
    assert!(!KindCompatibility::check(table, EdgeKind::Inherits, SymbolKind::Class));
}

#[test]
fn rust_typeref_accepts_struct_enum_trait_alias() {
    let table = RUST_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(table, EdgeKind::TypeRef, SymbolKind::Struct));
    assert!(KindCompatibility::check(table, EdgeKind::TypeRef, SymbolKind::Enum));
    assert!(KindCompatibility::check(table, EdgeKind::TypeRef, SymbolKind::Trait));
    assert!(KindCompatibility::check(table, EdgeKind::TypeRef, SymbolKind::TypeAlias));
}

#[test]
fn rust_supertype_discovery_is_explicit() {
    assert_eq!(RUST_PROFILE.supertype_discovery, SupertypeDiscovery::Explicit);
}

#[test]
fn rust_async_wrappers_contain_future() {
    assert!(RUST_PROFILE.async_wrappers.contains(&"Future"));
}
