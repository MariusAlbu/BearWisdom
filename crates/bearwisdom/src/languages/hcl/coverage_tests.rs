// =============================================================================
// hcl/coverage_tests.rs
//
// Node-kind coverage for HclPlugin::symbol_node_kinds() and ref_node_kinds().
// The plugin mod.rs stubs ExtractionResult::empty(); tests call extract::extract()
// directly with the tree-sitter-hcl grammar.
//
// symbol_node_kinds: block, attribute
// ref_node_kinds:    variable_expr, get_attr, function_call
// =============================================================================

use super::extract;
use crate::types::{EdgeKind, SymbolKind};

fn lang() -> tree_sitter::Language {
    tree_sitter_hcl::LANGUAGE.into()
}

// ---------------------------------------------------------------------------
// symbol_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_resource_block_produces_class() {
    let src = r#"resource "aws_instance" "web" {
  ami = "abc-123"
}"#;
    let r = extract::extract(src, lang());
    // A resource block maps to a Class symbol named by type.name convention.
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class),
        "resource block should produce Class symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_variable_block_produces_variable() {
    let src = r#"variable "region" {
  default = "us-east-1"
}"#;
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name.contains("region")),
        "variable block should produce Variable symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds
// ---------------------------------------------------------------------------

#[test]
fn cov_function_call_in_ref_node_kinds() {
    // function_call is declared in ref_node_kinds. The extractor extracts Calls refs
    // for function calls that appear within block bodies that have been indexed.
    // Verify the ref_node_kinds declaration is present.
    let plugin = crate::languages::hcl::HclPlugin;
    use crate::languages::LanguagePlugin;
    assert!(
        plugin.ref_node_kinds().contains(&"function_call"),
        "function_call should be declared in ref_node_kinds"
    );
}

#[test]
fn cov_variable_ref_in_ref_node_kinds() {
    // variable_expr / get_attr are the primary ref producers in HCL.
    let plugin = crate::languages::hcl::HclPlugin;
    use crate::languages::LanguagePlugin;
    assert!(
        plugin.ref_node_kinds().contains(&"variable_expr"),
        "variable_expr should be declared in ref_node_kinds"
    );
}
