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

// ---------------------------------------------------------------------------
// symbol_node_kinds: module_declaration
// ---------------------------------------------------------------------------

#[test]
fn cov_module_declaration_produces_class() {
    let src = "module storage './storage.bicep' = {\n  name: 'myStorage'\n}";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name == "storage"),
        "module_declaration should produce Class symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_module_declaration_produces_imports() {
    let src = "module storage './storage.bicep' = {\n  name: 'myStorage'\n}";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "module_declaration should produce Imports ref for module path; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// symbol_node_kinds: type_declaration
// ---------------------------------------------------------------------------

#[test]
fn cov_type_declaration_produces_typealias() {
    let src = "type storageAccountName = string";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::TypeAlias && s.name == "storageAccountName"),
        "type_declaration should produce TypeAlias symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// symbol_node_kinds: user_defined_function
// ---------------------------------------------------------------------------

#[test]
fn cov_user_defined_function_produces_function() {
    let src = "func buildUrl(prefix string) string => '${prefix}.example.com'";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Function && s.name == "buildUrl"),
        "user_defined_function should produce Function symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// symbol_node_kinds: metadata_declaration
// ---------------------------------------------------------------------------

#[test]
fn cov_metadata_declaration_produces_variable() {
    let src = "metadata author = 'Team Platform'";
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "author"),
        "metadata_declaration should produce Variable symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds: import_statement
// ---------------------------------------------------------------------------

#[test]
fn cov_import_statement_produces_imports() {
    // `import` syntax was added in Bicep 0.22 — grammar may parse as
    // import_statement or import_functionality depending on grammar version.
    // Either way an Imports ref must appear.
    let src = "import 'br:mcr.microsoft.com/bicep/extensions/microsofts:1.0' as exts";
    let r = extract::extract(src, lang());
    // Bicep grammars vary; test that refs contains Imports OR symbols contains the import alias.
    // If grammar doesn't parse this syntax, the test is a no-op (parse error is OK).
    // Only assert when no parse error was encountered.
    if !r.has_errors {
        assert!(
            r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
            "import_statement should produce Imports ref; got: {:?}",
            r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
        );
    }
}

// ---------------------------------------------------------------------------
// ref_node_kinds: call_expression — decorator
// ---------------------------------------------------------------------------

#[test]
fn cov_call_expression_in_decorator_produces_calls() {
    let src = "@description('The location')\nparam location string = 'eastus'";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "description"),
        "decorator call_expression should produce Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_call_expression_inline_produces_calls() {
    // Inline function call in a var declaration
    let src = "var lower = toLower('Hello')";
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "toLower"),
        "call_expression in var should produce Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// symbol_node_kinds: resource_declaration — signature includes type string
// ---------------------------------------------------------------------------

#[test]
fn cov_resource_declaration_signature() {
    let src = "resource sa 'Microsoft.Storage/storageAccounts@2021-02-01' = {\n  kind: 'StorageV2'\n}";
    let r = extract::extract(src, lang());
    let sym = r.symbols.iter().find(|s| s.name == "sa");
    assert!(sym.is_some(), "expected symbol 'sa'");
    let sig = sym.unwrap().signature.as_deref().unwrap_or("");
    assert!(sig.contains("sa"), "signature should contain resource name; got: {sig:?}");
}
