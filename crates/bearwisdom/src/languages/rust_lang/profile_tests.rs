use super::RUST_PROFILE;
use crate::type_checker::profile::language_profile::{
    KindCompatibility, ModuleAnchor, ModuleAnchorBind, SupertypeDiscovery,
};
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn rust_profile_identity() {
    assert_eq!(RUST_PROFILE.id, "rust");
    assert_eq!(RUST_PROFILE.qname_separator, "::");
    assert_eq!(RUST_PROFILE.self_keywords, &["self", "Self"]);
}

#[test]
fn rust_calls_accepts_function_method_constructor_closure_bindings() {
    let table = RUST_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(
        table,
        EdgeKind::Calls,
        SymbolKind::Function
    ));
    assert!(KindCompatibility::check(
        table,
        EdgeKind::Calls,
        SymbolKind::Method
    ));
    assert!(KindCompatibility::check(
        table,
        EdgeKind::Calls,
        SymbolKind::Constructor
    ));
    assert!(KindCompatibility::check(
        table,
        EdgeKind::Calls,
        SymbolKind::Variable
    ));
    assert!(KindCompatibility::check(
        table,
        EdgeKind::Calls,
        SymbolKind::Parameter
    ));
    assert!(KindCompatibility::check(
        table,
        EdgeKind::Calls,
        SymbolKind::Test
    ));
}

#[test]
fn rust_calls_accepts_enum_member_for_variant_construction() {
    // `Some(x)` / `Ok(y)` are tuple-variant constructions written with call
    // syntax — the bare prelude variant binds to its `enum_member` symbol on a
    // Calls edge, so the ambient-package strategy can resolve it.
    let table = RUST_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(
        table,
        EdgeKind::Calls,
        SymbolKind::EnumMember
    ));
    assert!(KindCompatibility::check(
        table,
        EdgeKind::Instantiates,
        SymbolKind::EnumMember
    ));
    assert!(KindCompatibility::check(
        table,
        EdgeKind::TypeRef,
        SymbolKind::EnumMember
    ));
}

#[test]
fn rust_calls_accepts_struct_for_struct_literal_construction() {
    // `Point { x: 1 }` (`struct_expression`) emits both a Calls ref and a
    // TypeRef for `Point` at the same site — the Calls ref needs the same
    // target kind the TypeRef already admits.
    let table = RUST_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(
        table,
        EdgeKind::Calls,
        SymbolKind::Struct
    ));
}

#[test]
fn rust_inherits_accepts_trait_only() {
    let table = RUST_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(
        table,
        EdgeKind::Inherits,
        SymbolKind::Trait
    ));
    assert!(!KindCompatibility::check(
        table,
        EdgeKind::Inherits,
        SymbolKind::Struct
    ));
    assert!(!KindCompatibility::check(
        table,
        EdgeKind::Inherits,
        SymbolKind::Class
    ));
}

#[test]
fn rust_typeref_accepts_struct_enum_trait_alias() {
    let table = RUST_PROFILE.kind_compatible_table;
    assert!(KindCompatibility::check(
        table,
        EdgeKind::TypeRef,
        SymbolKind::Struct
    ));
    assert!(KindCompatibility::check(
        table,
        EdgeKind::TypeRef,
        SymbolKind::Enum
    ));
    assert!(KindCompatibility::check(
        table,
        EdgeKind::TypeRef,
        SymbolKind::Trait
    ));
    assert!(KindCompatibility::check(
        table,
        EdgeKind::TypeRef,
        SymbolKind::TypeAlias
    ));
}

#[test]
fn rust_supertype_discovery_is_explicit() {
    assert_eq!(
        RUST_PROFILE.supertype_discovery,
        SupertypeDiscovery::Explicit
    );
}

#[test]
fn rust_async_wrappers_contain_future() {
    assert!(RUST_PROFILE.async_wrappers.contains(&"Future"));
}

#[test]
fn rust_import_module_path_is_from_module_field() {
    // The Rust profile must use FromModuleField so `use crate_name::Foo` import
    // bindings (kind=Imports refs where r.module=Some("crate_name")) populate
    // the file context's import list with the crate path. Without this, the
    // imported_namespace rule cannot match bare type refs to their use-imported
    // crate symbols.
    assert!(matches!(
        RUST_PROFILE.imports.import_module_path,
        crate::type_checker::profile::language_profile::ImportModulePath::FromModuleField
    ));
}

#[test]
fn rust_module_anchor_binds_by_name_under_module_dir() {
    assert!(matches!(
        RUST_PROFILE.imports.module_anchor,
        ModuleAnchor::On(ModuleAnchorBind::ByNameUnderModuleDir)
    ));
    // Non-terminal: a missed anchor falls through to the scope / import / qname
    // binders rather than ending the ladder.
    assert!(!RUST_PROFILE.imports.module_anchor_terminal);
}
