use super::NIM_PROFILE;
use crate::type_checker::profile::language_profile::{ExtMatch, KindCompatibility};
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn nim_calls_row_accepts_object_construction_without_dropping_procs() {
    let t = NIM_PROFILE.kind_compatible_table;
    // `Foo(field: x)` object construction and `Slot(x)` distinct/type conversion
    // bind to the type declaration (struct / type_alias).
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Struct
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::TypeAlias
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Enum
    ));
    // A real proc call still resolves to the proc (no regression).
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Function
    ));
    assert!(!KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Variable
    ));
}

#[test]
fn nim_calls_row_accepts_enum_member() {
    let t = NIM_PROFILE.kind_compatible_table;
    // An enum value used call-syntactically binds to the EnumMember.
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::EnumMember
    ));
}

#[test]
fn nim_profile_identity_and_shadow_mode() {
    assert_eq!(NIM_PROFILE.id, "nim");
    assert!(NIM_PROFILE.has_generics);
}

#[test]
fn nim_binds_imports_to_file_named_externals() {
    // The former resolve_ref import-leaf / package external tiers drain to the
    // import-scoped external bind matched by file-stem / dir against imports.
    // The unconditional any-stdlib guess tier is intentionally NOT reproduced.
    assert!(NIM_PROFILE.imports.external_by_import.is_some());
    assert_eq!(NIM_PROFILE.imports.ext_match, ExtMatch::FileStemOrDir);
}
