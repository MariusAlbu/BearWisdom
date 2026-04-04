// =============================================================================
// bicep/coverage_tests.rs
//
// Node-kind coverage for BicepPlugin::symbol_node_kinds() and ref_node_kinds().
// The plugin mod.rs returns ExtractionResult::empty() pending grammar wiring;
// these tests call extract::extract() directly with the live grammar.
//
// symbol_node_kinds: resource_declaration, module_declaration,
//                   parameter_declaration, variable_declaration,
//                   output_declaration, type_declaration,
//                   user_defined_function, metadata_declaration
// ref_node_kinds:    import_statement, import_with_statement,
//                   import_functionality, using_statement, call_expression
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

fn lang() -> tree_sitter::Language {
    tree_sitter_bicep::LANGUAGE.into()
}

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_resource_declaration_produces_class() {
    let src = "resource sa 'Microsoft.Storage/storageAccounts@2021-02-01' = {\n  kind: 'StorageV2'\n}";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name == "sa"),
        "resource_declaration should produce Class symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_parameter_declaration_produces_variable() {
    let src = "param location string = 'eastus'";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "location"),
        "parameter_declaration should produce Variable; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_variable_declaration_produces_variable() {
    let src = "var storagePrefix = 'mystore'";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "storagePrefix"),
        "variable_declaration should produce Variable; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_output_declaration_produces_variable() {
    let src = "output storageId string = sa.id";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "storageId"),
        "output_declaration should produce Variable; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_using_statement_produces_imports() {
    let src = "using './types.bicep'";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "using_statement should produce Imports ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}
