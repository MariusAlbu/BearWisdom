use super::PRISMA_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn prisma_profile_identity_and_shadow_mode() {
    assert_eq!(PRISMA_PROFILE.id, "prisma");
}

#[test]
fn prisma_type_ref_accepts_model_kinds_rejects_others() {
    let t = PRISMA_PROFILE.kind_compatible_table;
    for k in [
        SymbolKind::Struct,
        SymbolKind::Enum,
        SymbolKind::Class,
        SymbolKind::TypeAlias,
    ] {
        assert!(KindCompatibility::check(t, EdgeKind::TypeRef, k));
    }
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Function
    ));
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Variable
    ));
}
