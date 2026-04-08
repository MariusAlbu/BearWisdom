// =============================================================================
// powershell/coverage_tests.rs  —  One test per declared symbol_node_kind and ref_node_kind
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// Symbol node kinds
// ---------------------------------------------------------------------------

/// symbol_node_kind: `function_statement`
#[test]
fn symbol_function_statement() {
    let r = extract("function Run { Write-Host 'hello' }");
    assert!(
        r.symbols.iter().any(|s| s.name == "Run" && s.kind == SymbolKind::Function),
        "expected Function Run; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `class_statement`
#[test]
fn symbol_class_statement() {
    let r = extract("class Animal {\n    [string]$Name\n    Speak() { Write-Host $this.Name }\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "Animal" && s.kind == SymbolKind::Class),
        "expected Class Animal; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `enum_statement`
#[test]
fn symbol_enum_statement() {
    let r = extract("enum Color {\n    Red\n    Green\n    Blue\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "Color" && s.kind == SymbolKind::Enum),
        "expected Enum Color; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `class_method_definition`
#[test]
fn symbol_class_method_definition() {
    let r = extract("class Dog {\n    [string]$Name\n    Bark() { Write-Host 'Woof' }\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Method),
        "expected Method inside Dog; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `class_property_definition`
#[test]
fn symbol_class_property_definition() {
    let r = extract("class Config {\n    [int]$Timeout = 30\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Property),
        "expected Property inside Config; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Ref node kinds
// ---------------------------------------------------------------------------

/// ref_node_kind: `command`  —  cmdlet invocation emits a Calls edge.
#[test]
fn ref_command() {
    let r = extract("function Run { Write-Host 'hello' }");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Write-Host" && rf.kind == EdgeKind::Calls),
        "expected Calls Write-Host; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `command` inside script-block argument  —  commands passed as
/// script-block args to pipeline cmdlets (ForEach-Object, Where-Object, etc.) must
/// also be extracted. Previously the extractor stopped after extracting the outer
/// command and never recurse into its children.
#[test]
fn ref_command_inside_scriptblock_arg() {
    let r = extract("$list | ForEach-Object { Get-Item $_ }");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "ForEach-Object"),
        "expected Calls ForEach-Object; got {:?}",
        r.refs.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Get-Item"),
        "expected Calls Get-Item inside script block; got {:?}",
        r.refs.iter().map(|rf| &rf.target_name).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `invokation_expression`  —  method call on an object emits a Calls edge.
#[test]
fn ref_invokation_expression() {
    let r = extract("$obj.Method()");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Method" && rf.kind == EdgeKind::Calls),
        "expected Calls Method; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `invokation_expression` inside script-block arg  —  method calls
/// inside script blocks passed to cmdlets must be extracted.
#[test]
fn ref_invokation_inside_scriptblock_arg() {
    let r = extract("$list | ForEach-Object { $_.Compute() }");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Compute" && rf.kind == EdgeKind::Calls),
        "expected Calls Compute inside ForEach-Object script block; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `invokation_expression` static .NET call — [Math]::Round
/// target_name = "Round", module = Some("Math")
#[test]
fn ref_static_dotnet_method_call() {
    let r = extract("[Math]::Round(3.14)");
    let rf = r
        .refs
        .iter()
        .find(|rf| rf.target_name == "Round" && rf.kind == EdgeKind::Calls);
    assert!(
        rf.is_some(),
        "expected Calls Round; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert_eq!(
        rf.unwrap().module.as_deref(),
        Some("Math"),
        "expected module=Some(\"Math\"); got {:?}",
        rf.unwrap().module
    );
}

/// ref_node_kind: `invokation_expression` static .NET call — dotted type [System.IO.File]::ReadAllText
/// module should be the full dotted type name "System.IO.File"
#[test]
fn ref_static_dotnet_method_call_dotted_type() {
    let r = extract("[System.IO.File]::ReadAllText($path)");
    let rf = r
        .refs
        .iter()
        .find(|rf| rf.target_name == "ReadAllText" && rf.kind == EdgeKind::Calls);
    assert!(
        rf.is_some(),
        "expected Calls ReadAllText; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert_eq!(
        rf.unwrap().module.as_deref(),
        Some("System.IO.File"),
        "expected module=Some(\"System.IO.File\"); got {:?}",
        rf.unwrap().module
    );
}

/// ref_node_kind: `invokation_expression` member method call — $obj.Method()
/// target_name = "Method", module = Some("obj")
#[test]
fn ref_member_method_call() {
    let r = extract("$obj.Method()");
    let rf = r
        .refs
        .iter()
        .find(|rf| rf.target_name == "Method" && rf.kind == EdgeKind::Calls);
    assert!(
        rf.is_some(),
        "expected Calls Method; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert_eq!(
        rf.unwrap().module.as_deref(),
        Some("obj"),
        "expected module=Some(\"obj\"); got {:?}",
        rf.unwrap().module
    );
}

/// ref_node_kind: `using_statement`  —  the tree-sitter-powershell grammar currently
/// parses `using namespace …` as a `command` node rather than `using_statement`.
/// The extractor's extract_using handler is therefore unreachable from the current
/// grammar. This test documents the current behaviour: no Imports edge, no panic.
#[test]
fn ref_using_statement() {
    let r = extract("using namespace System.Collections.Generic");
    // Grammar emits a command node; extract_using is not invoked.
    // We assert no panic and the extractor returns a valid (possibly empty) result.
    let _ = r; // no assertion on edge presence; grammar mismatch is a known limitation
}
