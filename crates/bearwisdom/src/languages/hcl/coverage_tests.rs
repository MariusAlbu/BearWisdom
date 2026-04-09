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
// Resolution: extractor emits compound ref targets for Terraform patterns
// ---------------------------------------------------------------------------

#[test]
fn res_var_ref_emits_prefixed_target() {
    // var.instance_type → single ref with target "var.instance_type"
    let src = r#"output "t" { value = var.instance_type }"#;
    let r = extract::extract(src, lang());
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::TypeRef)
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"var.instance_type"),
        "var.instance_type should emit single target 'var.instance_type'; got: {:?}",
        type_refs
    );
    // Must NOT emit the fragments "var" or "instance_type" separately
    assert!(
        !type_refs.contains(&"var"),
        "fragmented 'var' ref must not be emitted; got: {:?}",
        type_refs
    );
}

#[test]
fn res_local_ref_emits_prefixed_target() {
    let src = r#"output "e" { value = local.env }"#;
    let r = extract::extract(src, lang());
    let targets: Vec<&str> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::TypeRef)
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(
        targets.contains(&"local.env"),
        "local.env should emit 'local.env'; got: {:?}",
        targets
    );
}

#[test]
fn res_module_ref_emits_module_name() {
    // module.vpc.vpc_id → emit "module.vpc" (the Namespace symbol qname)
    let src = r#"output "v" { value = module.vpc.vpc_id }"#;
    let r = extract::extract(src, lang());
    let targets: Vec<&str> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::TypeRef)
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(
        targets.contains(&"module.vpc"),
        "module.vpc.vpc_id should emit 'module.vpc'; got: {:?}",
        targets
    );
}

#[test]
fn res_data_ref_emits_data_qname() {
    // data.aws_ami.ubuntu.id → emit "data.aws_ami.ubuntu" (the Class symbol qname)
    let src = r#"resource "r" "n" { ami = data.aws_ami.ubuntu.id }"#;
    let r = extract::extract(src, lang());
    let targets: Vec<&str> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::TypeRef)
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(
        targets.contains(&"data.aws_ami.ubuntu"),
        "data.aws_ami.ubuntu.id should emit 'data.aws_ami.ubuntu'; got: {:?}",
        targets
    );
    // Must not emit the fragments "data", "aws_ami", "ubuntu", "id"
    for frag in &["data", "aws_ami", "ubuntu", "id"] {
        assert!(
            !targets.contains(frag),
            "fragment {:?} must not appear as a standalone ref; got: {:?}",
            frag,
            targets
        );
    }
}

#[test]
fn res_resource_cross_ref_emits_type_name() {
    // aws_instance.web.public_ip → emit "aws_instance.web"
    let src = r#"output "ip" { value = aws_instance.web.public_ip }"#;
    let r = extract::extract(src, lang());
    let targets: Vec<&str> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::TypeRef)
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(
        targets.contains(&"aws_instance.web"),
        "aws_instance.web.public_ip should emit 'aws_instance.web'; got: {:?}",
        targets
    );
}

#[test]
fn res_each_and_count_refs_not_emitted() {
    // each.value / count.index are meta — no TypeRef should be emitted for them
    let src = r#"resource "aws_subnet" "s" {
  for_each   = var.azs
  cidr_block = each.value
  index      = count.index
}"#;
    let r = extract::extract(src, lang());
    let targets: Vec<&str> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::TypeRef)
        .map(|rf| rf.target_name.as_str())
        .collect();
    for meta in &["each", "count", "each.value", "count.index"] {
        assert!(
            !targets.contains(meta),
            "meta-ref {:?} must not be emitted as TypeRef; got: {:?}",
            meta,
            targets
        );
    }
    // var.azs should still be emitted
    assert!(
        targets.contains(&"var.azs"),
        "var.azs should still be emitted; got: {:?}",
        targets
    );
}

#[test]
fn res_function_call_still_emits_calls() {
    // function calls inside expressions must still produce Calls refs
    let src = r#"locals { out = tostring(var.count) }"#;
    let r = extract::extract(src, lang());
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Calls && rf.target_name == "tostring"),
        "tostring() should still emit Calls ref; got: {:?}",
        r.refs.iter().map(|rf| (rf.kind, &rf.target_name)).collect::<Vec<_>>()
    );
    // var.count should also be emitted
    let type_refs: Vec<&str> = r
        .refs
        .iter()
        .filter(|rf| rf.kind == EdgeKind::TypeRef)
        .map(|rf| rf.target_name.as_str())
        .collect();
    assert!(
        type_refs.contains(&"var.count"),
        "var.count inside function arg should still emit TypeRef; got: {:?}",
        type_refs
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
