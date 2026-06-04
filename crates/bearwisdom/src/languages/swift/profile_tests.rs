use super::SWIFT_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn swift_profile_identity() {
    assert_eq!(SWIFT_PROFILE.id, "swift");
    assert_eq!(SWIFT_PROFILE.self_keywords, &["self", "Self", "super"]);
}

#[test]
fn swift_instantiates_accepts_class_struct_enum() {
    let t = SWIFT_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(t, EdgeKind::Instantiates, SymbolKind::Class));
    assert!(KindCompatibility::check(t, EdgeKind::Instantiates, SymbolKind::Struct));
    assert!(KindCompatibility::check(t, EdgeKind::Instantiates, SymbolKind::Enum));
}

#[test]
fn swift_implements_accepts_protocol_kinds() {
    let t = SWIFT_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(t, EdgeKind::Implements, SymbolKind::Interface));
    assert!(KindCompatibility::check(t, EdgeKind::Implements, SymbolKind::Trait));
}
