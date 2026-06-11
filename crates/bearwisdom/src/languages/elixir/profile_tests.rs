use super::ELIXIR_PROFILE;
use crate::type_checker::profile::language_profile::KindCompatibility;
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn elixir_profile_identity() {
    assert_eq!(ELIXIR_PROFILE.id, "elixir");
    assert_eq!(ELIXIR_PROFILE.qname_separator, ".");
}

#[test]
fn elixir_builtin_skip_declines_special_forms_not_user_functions() {
    // Kernel special forms and the def-family/control macros are language
    // constructs, not project symbols — they decline before the ladder so a
    // user function named `def_thing` or a real call like `process/1` still
    // resolves through it.
    let skip = ELIXIR_PROFILE
        .builtin_skip
        .expect("elixir builtin_skip set");
    assert!(skip("defmodule"));
    assert!(skip("def"));
    assert!(skip("defp"));
    assert!(skip("defmacro"));
    assert!(skip("case"));
    assert!(skip("cond"));
    assert!(skip("with"));
    assert!(skip("for"));
    assert!(skip("quote"));
    assert!(skip("unquote"));
    assert!(skip("if"));
    assert!(skip("unless"));
    // A user-defined function is NOT a language form — must resolve normally.
    assert!(!skip("process"));
    assert!(!skip("my_function"));
    assert!(!skip("def_thing"));
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
