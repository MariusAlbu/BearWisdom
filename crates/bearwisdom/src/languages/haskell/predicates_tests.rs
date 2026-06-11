use super::is_haskell_prelude_builtin;

#[test]
fn prelude_builtins_decline() {
    // Prelude primitive types, data constructors, functions, and operators are
    // implicitly in scope — they decline before the ladder.
    assert!(is_haskell_prelude_builtin("map"));
    assert!(is_haskell_prelude_builtin("filter"));
    assert!(is_haskell_prelude_builtin("Just"));
    assert!(is_haskell_prelude_builtin("Maybe"));
    assert!(is_haskell_prelude_builtin("<>"));
}

#[test]
fn named_library_types_do_not_decline() {
    // Library types/typeclasses are absent from the Prelude set so they resolve
    // to indexed externals rather than being declined as language primitives.
    assert!(!is_haskell_prelude_builtin("Text"));
    assert!(!is_haskell_prelude_builtin("Map"));
    assert!(!is_haskell_prelude_builtin("ToJSON"));
    assert!(!is_haskell_prelude_builtin("ReaderT"));
}
