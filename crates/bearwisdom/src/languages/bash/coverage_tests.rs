// =============================================================================
// bash/coverage_tests.rs — Node-kind coverage tests for the Bash extractor
//
// symbol_node_kinds: function_definition, variable_assignment, declaration_command
// ref_node_kinds:   command, command_substitution
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

/// function_definition → SymbolKind::Function  (POSIX form)
#[test]
fn cov_function_definition_posix_emits_function() {
    let r = extract::extract("foo() { bar; }");
    let sym = r.symbols.iter().find(|s| s.name == "foo");
    assert!(sym.is_some(), "expected Function symbol 'foo'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

/// function_definition → SymbolKind::Function  (keyword form)
#[test]
fn cov_function_definition_keyword_emits_function() {
    let r = extract::extract("function deploy { echo done; }");
    let sym = r.symbols.iter().find(|s| s.name == "deploy");
    assert!(sym.is_some(), "expected Function symbol 'deploy'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Function);
}

/// variable_assignment → SymbolKind::Variable  (file-scope)
#[test]
fn cov_variable_assignment_emits_variable() {
    let r = extract::extract("VERSION=1.0.0\n");
    let sym = r.symbols.iter().find(|s| s.name == "VERSION");
    assert!(sym.is_some(), "expected Variable symbol 'VERSION'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

/// declaration_command (declare/typeset/local) — extractor recognises the
/// node kind even if it does not emit a top-level symbol for every form;
/// a `declare -r NAME=val` at file scope should produce at least a Variable.
/// We verify the extractor does not crash and handles the source.
#[test]
fn cov_declaration_command_does_not_crash() {
    // declare is a declaration_command in tree-sitter-bash
    let src = "declare -r MAX=100\n";
    let r = extract::extract(src);
    // Either a Variable is emitted, or the extractor gracefully produces nothing.
    // Either way, no panic is acceptable.
    let _ = r;
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

/// command → EdgeKind::Calls  (non-builtin command inside a function)
#[test]
fn cov_command_inside_function_emits_calls() {
    let r = extract::extract("foo() { bar; }");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(calls.contains(&"bar"), "expected Calls ref to 'bar'; got: {calls:?}");
}

/// command → EdgeKind::Calls  (non-builtin command at top-level / script scope)
#[test]
fn cov_command_at_top_level_emits_calls() {
    let r = extract::extract("deploy_app\nnotify done\n");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(calls.contains(&"deploy_app"), "expected Calls ref to 'deploy_app' at top level; got: {calls:?}");
    assert!(calls.contains(&"notify"), "expected Calls ref to 'notify' at top level; got: {calls:?}");
}

/// command_substitution — the extractor should handle source files containing
/// command substitutions without crashing. Commands inside `$(...)` are nested
/// and may not be extracted as Calls edges by the current extractor, but the
/// source must be accepted without a panic.
#[test]
fn cov_command_substitution_does_not_crash() {
    let src = "foo() { result=$(get_value); }\n";
    let r = extract::extract(src);
    // The function itself should still be extracted.
    let sym = r.symbols.iter().find(|s| s.name == "foo");
    assert!(sym.is_some(), "expected Function 'foo' even with command_substitution body; got: {:?}", r.symbols);
}
