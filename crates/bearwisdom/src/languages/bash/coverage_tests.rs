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
fn cov_variable_assignment_file_scope_emits_variable() {
    let r = extract::extract("VERSION=1.0.0\n");
    let sym = r.symbols.iter().find(|s| s.name == "VERSION");
    assert!(sym.is_some(), "expected Variable symbol 'VERSION'; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

/// variable_assignment inside an if_statement → SymbolKind::Variable
#[test]
fn cov_variable_assignment_inside_if_emits_variable() {
    let r = extract::extract("if [ -n \"$X\" ]; then\n    RESULT=ok\nfi\n");
    let sym = r.symbols.iter().find(|s| s.name == "RESULT");
    assert!(sym.is_some(), "expected Variable 'RESULT' inside if; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

/// variable_assignment inside a for loop → SymbolKind::Variable
#[test]
fn cov_variable_assignment_inside_for_emits_variable() {
    let r = extract::extract("for i in 1 2 3; do\n    IDX=$i\ndone\n");
    let sym = r.symbols.iter().find(|s| s.name == "IDX");
    assert!(sym.is_some(), "expected Variable 'IDX' inside for; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

/// variable_assignment inside a function body → SymbolKind::Variable
#[test]
fn cov_variable_assignment_inside_function_emits_variable() {
    let r = extract::extract("setup() {\n    SETUP_DONE=1\n}\n");
    let sym = r.symbols.iter().find(|s| s.name == "SETUP_DONE");
    assert!(sym.is_some(), "expected Variable 'SETUP_DONE' inside function; got: {:?}", r.symbols);
}

/// declaration_command with declare → extracts the variable_assignment child
#[test]
fn cov_declaration_command_declare_emits_variable() {
    let r = extract::extract("declare -r MAX=100\n");
    let sym = r.symbols.iter().find(|s| s.name == "MAX");
    assert!(sym.is_some(), "expected Variable 'MAX' from declare; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

/// declaration_command with export → extracts the variable_assignment child
#[test]
fn cov_declaration_command_export_emits_variable() {
    let r = extract::extract("export PATH_EXT=/usr/local/bin\n");
    let sym = r.symbols.iter().find(|s| s.name == "PATH_EXT");
    assert!(sym.is_some(), "expected Variable 'PATH_EXT' from export; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

/// declaration_command with local inside a function → extracts variable
#[test]
fn cov_declaration_command_local_inside_function_emits_variable() {
    let r = extract::extract("setup() {\n    local TMPDIR=/tmp/work\n}\n");
    let sym = r.symbols.iter().find(|s| s.name == "TMPDIR");
    assert!(sym.is_some(), "expected Variable 'TMPDIR' from local; got: {:?}", r.symbols);
    assert_eq!(sym.unwrap().kind, SymbolKind::Variable);
}

/// declaration_command with no assignment (e.g. `export VAR`) — should not crash
#[test]
fn cov_declaration_command_no_assignment_does_not_crash() {
    let src = "export GIT_CONFIG_NOSYSTEM\n";
    let r = extract::extract(src);
    // No variable_assignment child → no symbol, but no crash
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

/// command → EdgeKind::Calls  (command at top-level / script scope)
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

/// command → EdgeKind::Calls for common external tools (git, make, curl)
#[test]
fn cov_command_external_tools_emit_calls() {
    let r = extract::extract("git status\nmake clean\ncurl -s https://example.com\n");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(calls.contains(&"git"), "expected Calls ref to 'git'; got: {calls:?}");
    assert!(calls.contains(&"make"), "expected Calls ref to 'make'; got: {calls:?}");
    assert!(calls.contains(&"curl"), "expected Calls ref to 'curl'; got: {calls:?}");
}

/// command inside a for loop → EdgeKind::Calls
#[test]
fn cov_command_inside_for_loop_emits_calls() {
    let r = extract::extract("for f in *.sh; do\n    process_file \"$f\"\ndone\n");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(calls.contains(&"process_file"), "expected Calls ref to 'process_file'; got: {calls:?}");
}

/// command_substitution — command inside `$(...)` emits a Calls ref
#[test]
fn cov_command_substitution_emits_calls() {
    let r = extract::extract("result=$(get_value)\n");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(calls.contains(&"get_value"), "expected Calls ref to 'get_value' from $(...); got: {calls:?}");
}

/// command_substitution inside a function body → Calls ref
#[test]
fn cov_command_substitution_inside_function_emits_calls() {
    let r = extract::extract("foo() { result=$(compute_value); }\n");
    let calls: Vec<&str> = r
        .refs
        .iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name.as_str())
        .collect();
    assert!(
        calls.contains(&"compute_value"),
        "expected Calls ref to 'compute_value' inside function $(...); got: {calls:?}"
    );
}

/// The function symbol itself should still be extracted when body has command_substitution
#[test]
fn cov_command_substitution_does_not_prevent_function_extraction() {
    let src = "foo() { result=$(get_value); }\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.name == "foo");
    assert!(sym.is_some(), "expected Function 'foo' even with command_substitution body; got: {:?}", r.symbols);
}
/// declaration_command with bare export (no assignment) → extracts variable by name
#[test]
fn cov_declaration_command_bare_export_emits_variable() {
    let r = extract::extract("export GITEA_TEST_E2E_DOMAIN
export GITEA_TEST_E2E_URL
");
    // Each bare export should emit a Variable symbol at its line
    let domain = r.symbols.iter().find(|s| s.name == "GITEA_TEST_E2E_DOMAIN");
    assert!(domain.is_some(), "expected Variable 'GITEA_TEST_E2E_DOMAIN' from bare export; got: {:?}", r.symbols);
    assert_eq!(domain.unwrap().kind, SymbolKind::Variable);
}
