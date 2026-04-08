// =============================================================================
// gleam/coverage_tests.rs
//
// Node-kind coverage for GleamPlugin::symbol_node_kinds() and ref_node_kinds().
// symbol_node_kinds: function, external_function, type_definition,
//                   type_alias, constant, import
// ref_node_kinds:    function_call, import, binary_expression
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_function_produces_function_symbol() {
    let r = extract::extract("pub fn add(a, b) { a + b }");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "add"),
        "pub fn should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_private_function_produces_function_symbol() {
    let r = extract::extract("fn helper(x) { x }");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "helper"),
        "fn should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_type_definition_produces_enum() {
    // `pub type` with constructors → Enum (ADT / custom type)
    let r = extract::extract("pub type Color {\n  Red\n  Green\n  Blue\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Enum && s.name == "Color"),
        "pub type should produce Enum symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_type_alias_produces_type_alias() {
    let r = extract::extract("pub type Name = String");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::TypeAlias && s.name == "Name"),
        "pub type alias should produce TypeAlias; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_constant_produces_variable() {
    let r = extract::extract("pub const max_size = 100");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "max_size"),
        "pub const should produce Variable symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_import_produces_imports_ref() {
    let r = extract::extract("import gleam/list");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name.contains("list")),
        "import should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_pipeline_produces_calls_ref() {
    // `|>` pipeline → binary_expression → Calls edge for the function on the RHS
    let r = extract::extract("pub fn run() {\n  [1, 2] |> list.map(double)\n}");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls),
        "pipeline (|>) should produce Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Additional symbol node kinds from rules
// ---------------------------------------------------------------------------

/// external_function — FFI binding should produce a Function symbol.
#[test]
fn cov_external_function_produces_function_symbol() {
    let r = extract::extract(
        "@external(erlang, \"erlang\", \"send\")\npub fn send(pid: a, msg: b) -> c",
    );
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "send"),
        "external_function should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// data_constructor — constructor variant inside type_definition → EnumMember.
#[test]
fn cov_data_constructor_produces_enum_member() {
    let r = extract::extract("pub type Color {\n  Red\n  Green\n  Blue\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::EnumMember && s.name == "Red"),
        "data_constructor should produce EnumMember symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// external_type — opaque FFI type binding → TypeAlias.
#[test]
fn cov_external_type_produces_type_alias() {
    let r = extract::extract("@external(erlang, \"erlang\", \"pid\")\npub type Pid");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::TypeAlias && s.name == "Pid"),
        "external_type should produce TypeAlias symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Private type_definition — no `pub` → still emits Enum symbol (private visibility).
#[test]
fn cov_private_type_definition_produces_enum() {
    let r = extract::extract("type Direction {\n  North\n  South\n}");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Enum && s.name == "Direction"),
        "private type should produce Enum symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Private constant — no `pub` → still emits Variable symbol.
#[test]
fn cov_private_constant_produces_variable() {
    let r = extract::extract("const default_timeout = 5000");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "default_timeout"),
        "private const should produce Variable symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Additional ref node kinds from rules
// ---------------------------------------------------------------------------

/// function_call direct (non-pipeline) → Calls ref.
#[test]
fn cov_function_call_produces_calls_ref() {
    let r = extract::extract("pub fn greet() {\n  io.println(\"hello\")\n}");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "println"),
        "function_call should produce Calls ref for 'println'; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

/// function_call where callee is field_access (module.function) → Calls ref using function name.
#[test]
fn cov_qualified_function_call_produces_calls_ref() {
    let r = extract::extract("pub fn run() {\n  string.length(\"hello\")\n}");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "length"),
        "module.function call should produce Calls ref for 'length'; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

/// import with unqualified names — still produces a single Imports ref for the module.
#[test]
fn cov_import_with_unqualified_names_produces_imports_ref() {
    let r = extract::extract("import gleam/list.{map, filter}");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports && rf.target_name.contains("list")),
        "import with unqualified names should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

/// import with alias — `import gleam/list as l` — Imports ref for the module.
#[test]
fn cov_import_with_alias_produces_imports_ref() {
    let r = extract::extract("import gleam/list as l");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "aliased import should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
