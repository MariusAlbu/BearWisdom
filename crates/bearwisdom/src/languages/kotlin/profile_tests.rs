use super::KOTLIN_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn kotlin_profile_identity() {
    assert_eq!(KOTLIN_PROFILE.id, "kotlin");
    assert_eq!(KOTLIN_PROFILE.qname_separator, ".");
    assert_eq!(KOTLIN_PROFILE.self_keywords, &["this", "super"]);
}

#[test]
fn kotlin_profile_engine_primary_disabled() {
    assert!(!KOTLIN_PROFILE.engine_primary);
}

#[test]
fn kotlin_calls_accepts_function_method_constructor_property() {
    let t = KOTLIN_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::Function));
    assert!(KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::Method));
    assert!(KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::Constructor));
    assert!(KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::Property));
}

#[test]
fn kotlin_inherits_accepts_class_and_interface() {
    let t = KOTLIN_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(t, EdgeKind::Inherits, SymbolKind::Class));
    assert!(KindCompatibility::check(t, EdgeKind::Inherits, SymbolKind::Interface));
}

#[test]
fn kotlin_async_wrappers_contain_deferred_and_flow() {
    assert!(KOTLIN_PROFILE.async_wrappers.contains(&"Deferred"));
    assert!(KOTLIN_PROFILE.async_wrappers.contains(&"Flow"));
}
