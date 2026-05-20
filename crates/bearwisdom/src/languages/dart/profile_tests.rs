use super::DART_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn dart_profile_identity() {
    assert_eq!(DART_PROFILE.id, "dart");
    assert_eq!(DART_PROFILE.self_keywords, &["this", "super"]);
}

#[test]
fn dart_profile_engine_primary_disabled() {
    assert!(!DART_PROFILE.engine_primary);
}

#[test]
fn dart_implements_accepts_class_and_interface() {
    let t = DART_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(t, EdgeKind::Implements, SymbolKind::Class));
    assert!(KindCompatibility::check(t, EdgeKind::Implements, SymbolKind::Interface));
}

#[test]
fn dart_async_wrappers_contain_future_and_stream() {
    assert!(DART_PROFILE.async_wrappers.contains(&"Future"));
    assert!(DART_PROFILE.async_wrappers.contains(&"Stream"));
}
