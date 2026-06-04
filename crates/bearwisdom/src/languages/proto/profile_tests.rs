use super::PROTO_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn proto_profile_identity_and_shadow_mode() {
    assert_eq!(PROTO_PROFILE.id, "proto");
}

#[test]
fn proto_type_ref_accepts_message_enum_kinds() {
    let t = PROTO_PROFILE.kind_compatible_table;
    for k in [SymbolKind::Struct, SymbolKind::Enum, SymbolKind::Class] {
        assert!(KindCompatibility::check(t, EdgeKind::TypeRef, k));
    }
    assert!(!KindCompatibility::check(t, EdgeKind::TypeRef, SymbolKind::Function));
}
