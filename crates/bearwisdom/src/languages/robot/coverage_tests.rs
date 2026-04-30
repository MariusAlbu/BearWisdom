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

// ---------------------------------------------------------------------------
// ref_node_kinds: setting_statement — Resource and Variables imports
// ---------------------------------------------------------------------------

#[test]
fn cov_resource_setting_produces_imports() {
    let src = "*** Settings ***\nResource    common.robot\n";
    let r = extract::extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "common.robot"),
        "Resource setting should produce Imports(common.robot); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_variables_setting_produces_imports() {
    let src = "*** Settings ***\nVariables    my_vars.py\n";
    let r = extract::extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name == "my_vars.py"),
        "Variables setting should produce Imports(my_vars.py); got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// symbol_node_kinds: variable_definition — name stripping
// ---------------------------------------------------------------------------

#[test]
fn cov_scalar_variable_strips_delimiters() {
    let src = "*** Variables ***\n${HOST}    localhost\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Variable && s.name == "HOST");
    assert!(
        sym.is_some(),
        "scalar variable should strip ${{}} delimiters to produce name 'HOST'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_list_variable_strips_delimiters() {
    // @{LIST} → name should be "LIST"
    let src = "*** Variables ***\n@{ITEMS}    one    two    three\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Variable && s.name == "ITEMS");
    assert!(
        sym.is_some(),
        "list variable should strip @{{}} delimiters to produce name 'ITEMS'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_dict_variable_strips_delimiters() {
    // &{DICT} → name should be "DICT"
    let src = "*** Variables ***\n&{CONFIG}    key=value\n";
    let r = extract::extract(src);
    let sym = r.symbols.iter().find(|s| s.kind == SymbolKind::Variable && s.name == "CONFIG");
    assert!(
        sym.is_some(),
        "dict variable should strip &{{}} delimiters to produce name 'CONFIG'; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds: keyword_invocation — in keyword body
// ---------------------------------------------------------------------------

#[test]
fn cov_keyword_invocation_in_keyword_body_produces_calls() {
    let src = "*** Keywords ***\nSetup Database\n    Connect To DB    myhost\n    Log    Connected\n";
    let r = extract::extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "Connect To DB"),
        "keyword invocation in keyword body should produce Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_keyword_invocation_assignment_pattern_produces_calls() {
    // ${result} =    Get Title  — keyword is the second cell
    let src = "*** Test Cases ***\nCheck Title\n    ${title} =    Get Title\n";
    let r = extract::extract(src);
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "Get Title"),
        "assignment-pattern keyword invocation should produce Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// symbol_node_kinds: multiple test cases and keywords in one file
// ---------------------------------------------------------------------------

#[test]
fn cov_multiple_test_cases() {
    let src = "*** Test Cases ***\nFirst Test\n    Log    one\nSecond Test\n    Log    two\n";
    let r = extract::extract(src);
    let tests: Vec<_> = r.symbols.iter().filter(|s| s.kind == SymbolKind::Test).collect();
    assert_eq!(tests.len(), 2, "expected 2 Test symbols; got: {:?}", tests);
}

#[test]
fn cov_multiple_keywords() {
    let src = "*** Keywords ***\nKeyword One\n    Log    one\nKeyword Two\n    Log    two\n";
    let r = extract::extract(src);
    let kws: Vec<_> = r.symbols.iter().filter(|s| s.kind == SymbolKind::Function).collect();
    assert_eq!(kws.len(), 2, "expected 2 Function symbols; got: {:?}", kws);
}

// ---------------------------------------------------------------------------
// Non-call markers — `...` continuation, `\END`, `VAR` inline assignment
// ---------------------------------------------------------------------------

#[test]
fn cov_continuation_marker_does_not_produce_call() {
    // `...` extends the previous line's argument list. It is not a keyword call.
    let src = "*** Test Cases ***\nMulti Line\n    Log Many    one    two\n    ...    three    four\n";
    let r = extract::extract(src);
    assert!(
        !r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "..."),
        "`...` continuation marker must not produce a Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, rf.target_name.clone())).collect::<Vec<_>>()
    );
}

#[test]
fn cov_escaped_end_marker_does_not_produce_call() {
    // `\END` is the escaped form of END used in older FOR loop fixtures.
    let src = "*** Keywords ***\nLoop Things\n    FOR    ${i}    IN RANGE    3\n    \\END\n";
    let r = extract::extract(src);
    assert!(
        !r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "\\END"),
        "`\\END` escape must not produce a Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, rf.target_name.clone())).collect::<Vec<_>>()
    );
}

#[test]
fn cov_var_inline_assignment_does_not_produce_call() {
    // Robot 6+ inline variable assignment syntax: `VAR    ${name}    value`.
    // VAR is a control marker, not a keyword call.
    let src = "*** Test Cases ***\nUse Var\n    VAR    ${greeting}    hello\n    Log    ${greeting}\n";
    let r = extract::extract(src);
    assert!(
        !r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "VAR"),
        "`VAR` inline assignment must not produce a Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, rf.target_name.clone())).collect::<Vec<_>>()
    );
}
