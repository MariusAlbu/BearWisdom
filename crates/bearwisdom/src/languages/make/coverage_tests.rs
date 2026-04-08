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
use crate::types::{EdgeKind, SymbolKind};

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

// ---------------------------------------------------------------------------
// define_directive → Variable
// ---------------------------------------------------------------------------

#[test]
fn cov_define_directive_produces_variable() {
    let src = "define GREETING\nhello world\nendef\n";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "GREETING"),
        "define_directive should produce Variable 'GREETING'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// shell_assignment → Variable
// ---------------------------------------------------------------------------

#[test]
fn cov_shell_assignment_produces_variable() {
    let src = "GIT_HASH != git rev-parse --short HEAD\n";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "GIT_HASH"),
        "shell_assignment should produce Variable 'GIT_HASH'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// include_directive → Imports edge
// ---------------------------------------------------------------------------

#[test]
fn cov_include_directive_produces_imports() {
    let src = "include config.mk\n";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name.contains("config.mk")),
        "include_directive should produce Imports ref to 'config.mk'; got: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_silent_include_directive_produces_imports() {
    // -include does not error if file is missing but still emits an Imports ref
    let src = "-include local.mk\n";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "-include directive should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// rule.prerequisites → Calls edges
// ---------------------------------------------------------------------------

#[test]
fn cov_rule_prerequisites_produce_calls() {
    let src = "all: build test\n\t@echo done\n";
    let r = extract::extract(src, lang());
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::Calls)
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(
        calls.contains(&"build"),
        "prerequisite 'build' should produce Calls edge; got: {calls:?}"
    );
    assert!(
        calls.contains(&"test"),
        "prerequisite 'test' should produce Calls edge; got: {calls:?}"
    );
}

#[test]
fn cov_rule_single_prerequisite_produces_calls() {
    let src = "link: compile\n\tld -o app compile.o\n";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "compile"),
        "single prerequisite 'compile' should produce Calls edge; got: {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// pattern rule `%.o` → Function
// ---------------------------------------------------------------------------

#[test]
fn cov_pattern_rule_produces_function() {
    let src = "%.o: %.c\n\t$(CC) -c $< -o $@\n";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name.contains('%')),
        "pattern rule should produce Function with '%' in name; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// variable_assignment operators
// ---------------------------------------------------------------------------

#[test]
fn cov_variable_assignment_immediate_expand_produces_variable() {
    // `:=` (immediate expansion) is still a variable_assignment
    let src = "OBJS := main.o util.o\n";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "OBJS"),
        "':=' assignment should produce Variable 'OBJS'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_variable_assignment_conditional_produces_variable() {
    // `?=` only assigns if not already set
    let src = "PREFIX ?= /usr/local\n";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "PREFIX"),
        "'?=' assignment should produce Variable 'PREFIX'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_variable_assignment_append_produces_variable() {
    // `+=` appends to an existing variable
    let src = "CFLAGS += -Wall\n";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "CFLAGS"),
        "'+=' assignment should produce Variable 'CFLAGS'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}
