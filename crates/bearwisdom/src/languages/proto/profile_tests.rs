// =============================================================================
// proto/profile_tests.rs — profile-axis, kind-table, and full-ladder binds.
//
// Protobuf messages/enums share one flat package namespace across `.proto`
// files, so a bare cross-file message/enum type ref binds to its declaration.
// `namespaceless_global_type_lookup == Global` drives that bind via the
// dead-last first-match-by-name rung; a same-named ext: well-known-type stub
// declines and stays external.
// =============================================================================

use super::PROTO_PROFILE;
use crate::type_checker::profile::language_profile::{KindCompatibility, NamespaceScope};
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn proto_profile_identity_and_shadow_mode() {
    assert_eq!(PROTO_PROFILE.id, "proto");
}

#[test]
fn proto_namespaceless_global_is_on() {
    // Messages/enums are package-flat across `.proto` files, so a bare type ref
    // binds via the dead-last first-match-by-name rung.
    assert_eq!(
        PROTO_PROFILE.namespaceless_global_type_lookup,
        NamespaceScope::Global
    );
}

#[test]
fn proto_type_ref_accepts_message_enum_kinds() {
    let t = PROTO_PROFILE.kind_compatible_table;
    for k in [SymbolKind::Struct, SymbolKind::Enum, SymbolKind::Class] {
        assert!(KindCompatibility::check(t, EdgeKind::TypeRef, k));
    }
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Function
    ));
}

