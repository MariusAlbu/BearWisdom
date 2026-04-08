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

// ---------------------------------------------------------------------------
// symbol_node_kinds: block — data, output, module, provider, terraform, locals
// ---------------------------------------------------------------------------

#[test]
fn cov_data_block_produces_class() {
    let src = r#"data "aws_ami" "ubuntu" {
  most_recent = true
}"#;
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name.contains("ubuntu")),
        "data block should produce Class symbol with instance name; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_output_block_produces_variable() {
    let src = r#"output "instance_ip" {
  value = "10.0.0.1"
}"#;
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name.contains("instance_ip")),
        "output block should produce Variable symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_module_block_produces_namespace_and_imports() {
    let src = r#"module "vpc" {
  source = "./modules/vpc"
}"#;
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Namespace && s.name.contains("vpc")),
        "module block should produce Namespace symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "module block should produce Imports ref for source path; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_provider_block_produces_class() {
    let src = r#"provider "aws" {
  region = "us-east-1"
}"#;
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Class && s.name.contains("aws")),
        "provider block should produce Class symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_terraform_block_produces_namespace() {
    let src = r#"terraform {
  required_version = ">= 1.0"
}"#;
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Namespace && s.name == "terraform"),
        "terraform block should produce Namespace symbol; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn cov_locals_block_attributes_produce_variables() {
    let src = r#"locals {
  env  = "production"
  region = "us-east-1"
}"#;
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "env"),
        "locals block attributes should produce Variable symbols; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds: function_call — emits Calls
// ---------------------------------------------------------------------------

#[test]
fn cov_function_call_produces_calls_ref() {
    let src = r#"locals {
  encoded = jsonencode({ key = "val" })
}"#;
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "jsonencode"),
        "function_call should produce Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds: variable_expr — emits TypeRef
// ---------------------------------------------------------------------------

#[test]
fn cov_variable_expr_produces_typeref() {
    let src = r#"output "region_out" {
  value = var.region
}"#;
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef),
        "variable_expr should produce TypeRef; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// ref_node_kinds: get_attr — emits TypeRef
// ---------------------------------------------------------------------------

#[test]
fn cov_get_attr_produces_typeref() {
    let src = r#"output "ami_id" {
  value = aws_instance.web.ami
}"#;
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef),
        "get_attr chain should produce TypeRef; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// symbol_node_kinds: attribute — top-level attribute emits Variable
// ---------------------------------------------------------------------------

#[test]
fn cov_top_level_attribute_produces_variable() {
    let src = r#"target_scope = "subscription""#;
    let r = extract::extract(src, lang());
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Variable && s.name == "target_scope"),
        "top-level attribute should produce Variable; got: {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}
