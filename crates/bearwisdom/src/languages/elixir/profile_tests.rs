use super::ELIXIR_PROFILE;
use crate::type_checker::profile::language_profile::{ImportModulePath, KindCompatibility};
use crate::types::{EdgeKind, SymbolKind};

#[test]
fn elixir_profile_identity() {
    assert_eq!(ELIXIR_PROFILE.id, "elixir");
    assert_eq!(ELIXIR_PROFILE.qname_separator, ".");
}

#[test]
fn elixir_import_and_use_are_wildcards_via_module_field() {
    // `import`/`use` bring a whole module's surface into bare-name scope;
    // `directives.rs` marks only those two non-binding, so this flag widens
    // exactly them. `module_path` must come from the ref's own `module` field
    // (set to the directive's fully-qualified target) or the wildcard rung has
    // no module to search under.
    assert!(ELIXIR_PROFILE.imports.namespace_imports_are_wildcards);
    assert_eq!(
        ELIXIR_PROFILE.imports.import_module_path,
        ImportModulePath::FromModuleField
    );
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
