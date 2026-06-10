use super::ELIXIR_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn elixir_profile_identity() {
    assert_eq!(ELIXIR_PROFILE.id, "elixir");
    assert_eq!(ELIXIR_PROFILE.qname_separator, ".");
}

#[test]
fn elixir_kind_table_accepts_module_and_callable_kinds() {
    let t = ELIXIR_PROFILE.kind_compatible_table;
    // Module is a Calls target (defmodule functions live under a module).
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Function
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::Calls,
        SymbolKind::Module
    ));
    // TypeRef accepts module/type kinds.
    assert!(KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::Module
    ));
    assert!(KindCompatibility::check(
        t,
        EdgeKind::TypeRef,
        SymbolKind::TypeAlias
    ));
}
