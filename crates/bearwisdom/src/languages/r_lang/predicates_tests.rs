use super::is_r_builtin;

#[test]
fn r_language_primitives_decline() {
    // Control-flow keyword, language constant, and C-implemented interpreter
    // primitives are builtins — they decline before the ladder.
    assert!(is_r_builtin("function"));
    assert!(is_r_builtin("NULL"));
    assert!(is_r_builtin("is.null"));
    assert!(is_r_builtin("as.numeric"));
    assert!(is_r_builtin("UseMethod"));
}

#[test]
fn r_walker_resolved_base_functions_do_not_decline() {
    // Base/stats functions with real `.R` source are resolved by the r_stdlib
    // walker, not declined as builtins — they are absent from the closed set.
    assert!(!is_r_builtin("paste"));
    assert!(!is_r_builtin("sapply"));
    assert!(!is_r_builtin("ggplot"));
}
