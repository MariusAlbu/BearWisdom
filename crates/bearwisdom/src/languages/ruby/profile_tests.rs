use super::RUBY_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn ruby_profile_identity() {
    assert_eq!(RUBY_PROFILE.id, "ruby");
    assert_eq!(RUBY_PROFILE.qname_separator, "::");
    assert_eq!(RUBY_PROFILE.self_keywords, &["self"]);
}

#[test]
fn ruby_calls_accepts_method_function_constructor() {
    let t = RUBY_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::Method));
    assert!(KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::Function));
    assert!(KindCompatibility::check(t, EdgeKind::Calls, SymbolKind::Constructor));
}

#[test]
fn ruby_inherits_accepts_class_only() {
    let t = RUBY_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(t, EdgeKind::Inherits, SymbolKind::Class));
    assert!(!KindCompatibility::check(t, EdgeKind::Inherits, SymbolKind::Module));
}

#[test]
fn ruby_implements_accepts_module_or_interface() {
    let t = RUBY_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(t, EdgeKind::Implements, SymbolKind::Module));
    assert!(KindCompatibility::check(t, EdgeKind::Implements, SymbolKind::Interface));
}

#[test]
fn ruby_iterator_method_is_each() {
    assert_eq!(RUBY_PROFILE.iterator_method, Some("each"));
}
