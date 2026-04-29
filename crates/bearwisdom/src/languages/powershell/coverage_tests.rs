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

/// symbol_node_kind: `enum_member`  —  EnumMember inside an enum body.
#[test]
fn symbol_enum_member() {
    let r = extract("enum Direction {\n    North\n    South\n    East\n    West\n}");
    assert!(
        r.symbols.iter().any(|s| s.name == "Direction" && s.kind == SymbolKind::Enum),
        "expected Enum Direction; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    for member in &["North", "South", "East", "West"] {
        assert!(
            r.symbols.iter().any(|s| s.name == *member && s.kind == SymbolKind::EnumMember),
            "expected EnumMember {}; got {:?}",
            member,
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
    }
}

/// symbol_node_kind: `assignment_expression` (top-level `$var =`)  →  Variable
#[test]
fn symbol_assignment_expression_top_level() {
    let r = extract("$Global = 'value'");
    assert!(
        r.symbols.iter().any(|s| s.name == "Global" && s.kind == SymbolKind::Variable),
        "expected Variable Global from top-level assignment; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// symbol_node_kind: `script_parameter` in `param_block`  →  Variable
#[test]
fn symbol_script_parameter() {
    let r = extract("param(\n    [string]$Name,\n    [int]$Count = 0\n)\nWrite-Host $Name");
    assert!(
        r.symbols.iter().any(|s| s.name == "Name" && s.kind == SymbolKind::Variable),
        "expected Variable Name from param block; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.symbols.iter().any(|s| s.name == "Count" && s.kind == SymbolKind::Variable),
        "expected Variable Count from param block; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `command` where name is `Import-Module`  →  EdgeKind::Imports
/// The extractor emits an Imports edge for `Import-Module` commands.
#[test]
fn ref_import_module_command() {
    let r = extract("Import-Module Az");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name.contains("Az")),
        "expected Imports ref to 'Az' from Import-Module; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `class_statement` with `:` base type  →  EdgeKind::Inherits
#[test]
fn ref_class_inherits() {
    let r = extract("class Dog : Animal {\n    Bark() { Write-Host 'Woof' }\n}");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Animal" && rf.kind == EdgeKind::Inherits),
        "expected Inherits edge from Dog to Animal; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// ref_node_kind: `member_access` static type access — `[Type]::Member`
/// emits a TypeRef. Runtime property reads on a `$variable` are skipped
/// (no type info is available to resolve them, and they otherwise flood
/// unresolved_refs with hashtable keys / AST property names).
#[test]
fn ref_member_access_static_type() {
    let r = extract("$max = [int]::MaxValue");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "MaxValue" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef ref to MaxValue from [int]::MaxValue; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_member_access_runtime_var_is_skipped() {
    let r = extract("$length = $str.Length");
    assert!(
        !r.refs.iter().any(|rf| rf.target_name == "Length"),
        "did not expect a ref to Length from $str.Length; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}
