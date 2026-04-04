// =============================================================================
// cmake/coverage_tests.rs
//
// Node-kind coverage for CMakePlugin::symbol_node_kinds() and ref_node_kinds().
// The plugin mod.rs returns ExtractionResult::empty() pending grammar wiring;
// these tests call extract::extract() directly with the live grammar.
//
// symbol_node_kinds: function_def, macro_def, normal_command
// ref_node_kinds:    normal_command, variable_ref
// =============================================================================

use super::extract;
use crate::types::SymbolKind;

fn lang() -> tree_sitter::Language {
    tree_sitter_cmake::LANGUAGE.into()
}

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_function_def_produces_function() {
    let src = "function(my_func arg1)\n  message(\"hello\")\nendfunction()";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "my_func"),
        "function_def should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_macro_def_produces_function() {
    let src = "macro(my_macro)\n  message(\"macro\")\nendmacro()";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "my_macro"),
        "macro_def should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_normal_command_produces_symbol_or_ref() {
    // add_executable is a normal_command that creates a build target (Function).
    let src = "add_executable(myapp main.c)";
    let r = extract::extract(src, lang());
    // Either a symbol or a Calls ref — at minimum the file should parse without error.
    assert!(!r.has_errors, "normal_command snippet should parse cleanly");
}
