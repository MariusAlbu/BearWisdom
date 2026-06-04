use super::PHP_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn php_profile_identity() {
    assert_eq!(PHP_PROFILE.id, "php");
    assert_eq!(PHP_PROFILE.qname_separator, "\\");
}

#[test]
fn php_profile_self_keywords_cover_receiver_forms() {
    assert!(PHP_PROFILE.self_keywords.contains(&"$this"));
    assert!(PHP_PROFILE.self_keywords.contains(&"self"));
    assert!(PHP_PROFILE.self_keywords.contains(&"static"));
    assert!(PHP_PROFILE.self_keywords.contains(&"parent"));
}

#[test]
fn php_calls_accepts_function_method_constructor() {
    let t = PHP_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::Function));
    assert!(KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::Method));
    assert!(KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::Constructor));
}

#[test]
fn php_implements_accepts_interface_only() {
    let t = PHP_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(t, EdgeKind::Implements, SymbolKind::Interface));
    assert!(!KindCompatibility::check(t, EdgeKind::Implements, SymbolKind::Class));
}

#[test]
fn php_instantiates_accepts_class_only() {
    let t = PHP_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(t, EdgeKind::Instantiates, SymbolKind::Class));
    assert!(!KindCompatibility::check(t, EdgeKind::Instantiates, SymbolKind::Interface));
}
