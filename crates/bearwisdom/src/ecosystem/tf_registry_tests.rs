// =============================================================================
// ecosystem/tf_registry_tests.rs — sibling tests for tf_registry.rs
// =============================================================================

use super::*;
use crate::ecosystem::manifest::ManifestReader;

#[test]
fn ecosystem_identity() {
    let eco = TfRegistryEcosystem;
    assert_eq!(eco.id(), ID);
    assert_eq!(Ecosystem::kind(&eco), EcosystemKind::Package);
    assert_eq!(Ecosystem::languages(&eco), &["hcl"]);
}

#[test]
fn legacy_locator_tag() {
    assert_eq!(ExternalSourceLocator::ecosystem(&TfRegistryEcosystem), "tf-registry");
}

#[test]
fn extract_required_providers_from_versions_tf() {
    let src = r#"
terraform {
  required_version = ">= 1.0"

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = ">= 6.28"
    }
    random = {
      source = "hashicorp/random"
    }
  }
}
"#;
    let providers = extract_required_providers(src);
    assert!(
        providers.contains(&"hashicorp/aws".to_string()),
        "expected hashicorp/aws in {providers:?}"
    );
    assert!(
        providers.contains(&"hashicorp/random".to_string()),
        "expected hashicorp/random in {providers:?}"
    );
    assert_eq!(providers.len(), 2);
}

#[test]
fn extract_module_sources_skips_local_paths() {
    let src = r#"
module "vpc" {
  source  = "terraform-aws-modules/vpc/aws"
  version = "5.8.1"
}

module "local_util" {
  source = "./modules/util"
}
"#;
    let sources = extract_module_sources(src);
    assert!(
        sources.contains(&"terraform-aws-modules/vpc/aws".to_string()),
        "expected registry module in {sources:?}"
    );
    assert!(
        !sources.iter().any(|s| s.starts_with('.')),
        "local path leaked into module sources: {sources:?}"
    );
    assert_eq!(sources.len(), 1);
}

#[test]
fn scan_tf_top_level_resources_extracts_resource_and_data() {
    let src = r#"
resource "aws_vpc" "this" {
  cidr_block = "10.0.0.0/16"
}

data "aws_ami" "latest" {
  filter { }
}

module "vpc" {
  source = "terraform-aws-modules/vpc/aws"
}

output "vpc_id" {
  value = aws_vpc.this.id
}
"#;
    let syms = scan_tf_top_level_resources(src);
    assert!(syms.contains(&"aws_vpc.this".to_string()), "{syms:?}");
    assert!(syms.contains(&"data.aws_ami.latest".to_string()), "{syms:?}");
    assert!(syms.contains(&"module.vpc".to_string()), "{syms:?}");
    assert!(syms.contains(&"output.vpc_id".to_string()), "{syms:?}");
}

#[test]
fn synthesize_bundled_providers_covers_top3() {
    let files = _test_synthesize_bundled_providers();
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.iter().any(|p| p.contains(":aws/")), "aws provider missing: {paths:?}");
    assert!(paths.iter().any(|p| p.contains(":google/")), "google provider missing: {paths:?}");
    assert!(paths.iter().any(|p| p.contains(":azurerm/")), "azurerm provider missing: {paths:?}");
    let all_symbols: Vec<&str> = files
        .iter()
        .flat_map(|f| f.symbols.iter().map(|s| s.name.as_str()))
        .collect();
    for expected in ["aws_vpc", "aws_s3_bucket", "google_compute_instance", "azurerm_resource_group"] {
        assert!(
            all_symbols.contains(&expected),
            "symbol {expected} missing from bundled synthetics"
        );
    }
}

#[test]
fn bundled_providers_no_empty_symbols() {
    for pf in _test_synthesize_bundled_providers() {
        assert!(!pf.symbols.is_empty(), "provider file {} has no symbols", pf.path);
        for sym in &pf.symbols {
            assert!(!sym.name.is_empty(), "empty symbol name in {}", pf.path);
            assert_eq!(sym.kind, crate::types::SymbolKind::Class, "expected Class kind for {}", sym.name);
        }
    }
}

#[test]
fn parse_two_labels_handles_aligned_quotes() {
    assert_eq!(
        _test_parse_two_labels(r#""aws_vpc" "this" {"#),
        Some(("aws_vpc", "this"))
    );
}

#[test]
fn extract_source_value_handles_extra_spaces() {
    assert_eq!(_test_extract_source_value(r#"source  =  "hashicorp/aws""#), Some("hashicorp/aws"));
    assert_eq!(_test_extract_source_value(r#"source = """#), None);
    assert_eq!(_test_extract_source_value(r#"version = "1.0""#), None);
}

// ---------------------------------------------------------------------------
// New: TerraformManifest reader tests
// ---------------------------------------------------------------------------

#[test]
fn manifest_reader_unions_providers_and_module_sources() {
    let tmp = std::env::temp_dir().join("bw-tf-manifest-union");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(
        tmp.join("versions.tf"),
        r#"terraform {
  required_providers {
    aws = {
      source = "hashicorp/aws"
    }
  }
}
"#,
    ).unwrap();
    std::fs::write(
        tmp.join("main.tf"),
        r#"module "vpc" {
  source  = "terraform-aws-modules/vpc/aws"
  version = "5.8.1"
}
"#,
    ).unwrap();

    let data = TerraformManifest.read(&tmp).expect("manifest data present");
    assert!(data.dependencies.contains("hashicorp/aws"));
    assert!(data.dependencies.contains("terraform-aws-modules/vpc/aws"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn manifest_reader_returns_some_for_bare_tf_file_without_declarations() {
    // `.tf` presence IS the activation signal — even without
    // required_providers or module blocks the reader must surface a
    // (possibly empty-deps) ManifestData so the ecosystem activates and
    // the bundled provider synthetics are emitted.
    let tmp = std::env::temp_dir().join("bw-tf-manifest-bare");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(
        tmp.join("main.tf"),
        "resource \"aws_vpc\" \"this\" { cidr_block = \"10.0.0.0/16\" }\n",
    ).unwrap();

    let data = TerraformManifest.read(&tmp).expect("manifest data present");
    // No declarations to extract, but the .tf presence still activates.
    assert!(data.dependencies.is_empty());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn manifest_reader_returns_none_for_project_without_tf_files() {
    let tmp = std::env::temp_dir().join("bw-tf-manifest-empty");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("README.md"), "no terraform here").unwrap();
    assert!(TerraformManifest.read(&tmp).is_none());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn manifest_reader_walks_subdirectories() {
    // Provider declared in a nested module, module sources in root —
    // the reader must walk subdirs to union both.
    let tmp = std::env::temp_dir().join("bw-tf-manifest-nested");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("modules/networking")).unwrap();
    std::fs::write(
        tmp.join("main.tf"),
        r#"module "vpc" {
  source = "terraform-aws-modules/vpc/aws"
}
"#,
    ).unwrap();
    std::fs::write(
        tmp.join("modules/networking/versions.tf"),
        r#"terraform {
  required_providers {
    aws = {
      source = "hashicorp/aws"
    }
  }
}
"#,
    ).unwrap();

    let data = TerraformManifest.read(&tmp).expect("manifest data present");
    assert!(data.dependencies.contains("hashicorp/aws"));
    assert!(data.dependencies.contains("terraform-aws-modules/vpc/aws"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn manifest_reader_skips_dot_terraform_dir() {
    // The reader must not recurse into `.terraform/` — that's where
    // downloaded modules live; its `.tf` files are external code, not
    // project deps.
    let tmp = std::env::temp_dir().join("bw-tf-manifest-skip-dottf");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join(".terraform/modules/external")).unwrap();
    std::fs::write(
        tmp.join(".terraform/modules/external/main.tf"),
        r#"terraform { required_providers { leaked = { source = "should/not/leak" } } }"#,
    ).unwrap();
    std::fs::write(tmp.join("main.tf"), "resource \"aws_vpc\" \"this\" {}\n").unwrap();

    let data = TerraformManifest.read(&tmp).expect("manifest data present");
    assert!(
        !data.dependencies.contains("should/not/leak"),
        "deps from .terraform/ leaked into manifest: {:?}",
        data.dependencies
    );
    let _ = std::fs::remove_dir_all(&tmp);
}
