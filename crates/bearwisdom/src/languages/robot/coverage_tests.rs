// =============================================================================
// robot/coverage_tests.rs
//
// Node-kind coverage for RobotPlugin::symbol_node_kinds() and ref_node_kinds().
// Grammar returns None; extraction is performed by the section-aware line scanner.
//
// symbol_node_kinds: keyword_definition, test_case_definition, variable_definition
// ref_node_kinds:    keyword_invocation, setting_statement
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_test_case_definition_produces_test() {
    let src = "*** Test Cases ***\nMy Test\n    Log    Hello\n";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Test && s.name == "My Test"),
        "test case should produce Test symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_keyword_definition_produces_function() {
    let src = "*** Keywords ***\nGreet User\n    [Arguments]    ${name}\n    Log    Hello ${name}\n";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "Greet User"),
        "keyword definition should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_variable_definition_produces_variable() {
    let src = "*** Variables ***\n${HOST}    localhost\n";
    let r = extract::extract(src);
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable),
        "variable definition should produce Variable symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_keyword_invocation_in_test_case_produces_calls() {
    let src = "*** Test Cases ***\nSample\n    Log    Hello\n    Sleep    1s\n";
    let r = extract::extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "keyword invocations should produce Calls refs; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_library_setting_produces_imports() {
    let src = "*** Settings ***\nLibrary    Collections\n";
    let r = extract::extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "Collections"),
        "Library setting should produce Imports(Collections); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
