// =============================================================================
// make/coverage_tests.rs
//
// Node-kind coverage for MakePlugin::symbol_node_kinds() and ref_node_kinds().
// The plugin mod.rs stubs ExtractionResult::empty() pending grammar wiring;
// these tests call extract::extract() directly with the live grammar.
//
// symbol_node_kinds: rule, variable_assignment, define_directive, shell_assignment
// ref_node_kinds:    include_directive, function_call, shell_function
// =============================================================================

use super::extract;
use crate::types::SymbolKind;

fn lang() -> tree_sitter::Language {
    tree_sitter_make::LANGUAGE.into()
}

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_rule_produces_function() {
    // Make rule target → Function symbol
    let src = "build: src/main.c\n\tgcc -o build src/main.c\n";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "build"),
        "rule target should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_variable_assignment_produces_variable() {
    let src = "CC = gcc\n";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "CC"),
        "variable assignment should produce Variable; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_phony_rule_produces_function() {
    let src = ".PHONY: clean\nclean:\n\trm -f *.o\n";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "clean"),
        "phony rule should produce Function; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}
