// =============================================================================
// ecosystem/tf_registry.rs — Terraform Registry ecosystem
//
// Terraform projects reference two kinds of external symbols:
//
//   1. Provider resources — `resource "aws_vpc" "this" { ... }` and
//      `data "aws_ami" "latest" { ... }` reference provider-defined types
//      (e.g. `aws_vpc`, `aws_s3_bucket`). Providers ship as Go binaries, not
//      source. For MVP we bundle synthetic symbols for the top 3 providers
//      (aws, google, azurerm) covering the ~30 most common resource types.
//      When `.terraform/providers/...` exists on disk we could extend this via
//      schema.json; that path is left as a TODO for a future session.
//
//   2. Module calls — `module "x" { source = "terraform-aws-modules/vpc/aws" }`
//      can be resolved by walking `.terraform/modules/<key>/` after `terraform
//      init` has already been run. We walk those directories for `.tf` files.
//
// Activation: Any([ManifestMatch, LanguagePresent("hcl")]) — fires for any
// project with a `*.tf` file or a `terraform.tfvars`.
//
// We do NOT call `terraform init` at index time. If `.terraform/` is absent,
// we fall back to the bundled synthetic symbols only.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

pub const ID: EcosystemId = EcosystemId::new("tf-registry");
const LEGACY_ECOSYSTEM_TAG: &str = "tf-registry";
const LANGUAGES: &[&str] = &["hcl"];

// ---------------------------------------------------------------------------
// Manifest specs
// ---------------------------------------------------------------------------

fn parse_tf_manifest(path: &Path) -> std::io::Result<crate::ecosystem::manifest::ManifestData> {
    use crate::ecosystem::manifest::ManifestData;
    let content = std::fs::read_to_string(path)?;
    let providers = extract_required_providers(&content);
    let modules = extract_module_sources(&content);
    let mut data = ManifestData::default();
    data.module_path = path
        .parent()
        .and_then(|p| p.to_str())
        .map(|s| s.to_string());
    for p in providers {
        data.dependencies.insert(p);
    }
    for m in modules {
        data.dependencies.insert(m);
    }
    Ok(data)
}

const MANIFESTS: &[ManifestSpec] = &[
    ManifestSpec { glob: "**/versions.tf",       parse: parse_tf_manifest },
    ManifestSpec { glob: "**/main.tf",            parse: parse_tf_manifest },
    ManifestSpec { glob: "**/terraform.tfvars",   parse: parse_tf_manifest },
];

// ---------------------------------------------------------------------------
// Ecosystem struct
// ---------------------------------------------------------------------------

pub struct TfRegistryEcosystem;

impl Ecosystem for TfRegistryEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("hcl"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        locate_tf_roots(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_tf_module_dir(dep)
    }

    fn parse_metadata_only(&self, dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        if dep.module_path == "tf-providers-bundled" {
            Some(synthesize_bundled_providers())
        } else {
            None
        }
    }

    fn uses_demand_driven_parse(&self) -> bool { true }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_tf_symbol_index(dep_roots)
    }
}

impl ExternalSourceLocator for TfRegistryEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        locate_tf_roots(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_tf_module_dir(dep)
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        // Always emit bundled synthetics regardless of project root; callers
        // that want module-resolved files go through locate_roots + walk_root.
        Some(synthesize_bundled_providers())
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<TfRegistryEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(TfRegistryEcosystem)).clone()
}

// =============================================================================
// Root discovery
// =============================================================================

/// Return one ExternalDepRoot per downloaded Terraform module in
/// `.terraform/modules/`, plus a synthetic-only root for the bundled provider
/// symbols (which are emitted via `parse_metadata_only` rather than a real
/// walk).
pub fn locate_tf_roots(project_root: &Path) -> Vec<ExternalDepRoot> {
    let mut roots: Vec<ExternalDepRoot> = Vec::new();

    // --- synthetic provider symbols root (always present) ---
    roots.push(ExternalDepRoot {
        module_path: "tf-providers-bundled".to_string(),
        version: "bundled".to_string(),
        // The path is unused for this synthetic-only root; point at the
        // project root as a stable dummy so callers don't get confused.
        root: project_root.to_path_buf(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    });

    // --- downloaded module roots (requires terraform init to have run) ---
    let modules_json = project_root
        .join(".terraform")
        .join("modules")
        .join("modules.json");

    if modules_json.is_file() {
        if let Some(module_roots) = parse_modules_json(&modules_json) {
            roots.extend(module_roots);
        }
    }

    debug!(
        "tf-registry: {} dep roots for {}",
        roots.len(),
        project_root.display()
    );
    roots
}

/// Parse `.terraform/modules/modules.json` to discover downloaded module dirs.
///
/// Format:
/// ```json
/// { "Modules": [
///     { "Key": "vpc", "Source": "terraform-aws-modules/vpc/aws",
///       "Version": "5.8.1", "Dir": ".terraform/modules/vpc" },
///     ...
/// ]}
/// ```
fn parse_modules_json(path: &Path) -> Option<Vec<ExternalDepRoot>> {
    let bytes = std::fs::read(path).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let modules = json.get("Modules")?.as_array()?;
    let project_root = path.parent()?.parent()?.parent()?;

    let mut out = Vec::new();
    for m in modules {
        let key = m.get("Key")?.as_str()?;
        // Skip the implicit root module entry (empty key or key == "")
        if key.is_empty() { continue; }
        let dir = m.get("Dir")?.as_str()?;
        let source = m.get("Source").and_then(|v| v.as_str()).unwrap_or(key);
        let version = m.get("Version").and_then(|v| v.as_str()).unwrap_or("").to_string();

        // Dir is relative to the project root.
        let abs_dir = project_root.join(dir);
        if !abs_dir.is_dir() { continue; }

        out.push(ExternalDepRoot {
            module_path: source.to_string(),
            version,
            root: abs_dir,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        });
    }
    Some(out)
}

// =============================================================================
// Module file walk
// =============================================================================

/// Walk a downloaded Terraform module directory for `.tf` files.
fn walk_tf_module_dir(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    // The bundled-providers root has no source files to walk — symbols come
    // from parse_metadata_only.
    if dep.module_path == "tf-providers-bundled" {
        return Vec::new();
    }
    let mut out = Vec::new();
    walk_tf_dir(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_tf_dir(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth > 6 { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, ".terraform" | ".git" | "examples" | "tests" | "test")
                    || name.starts_with('.')
                {
                    continue;
                }
            }
            walk_tf_dir(&path, root, dep, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !name.ends_with(".tf") { continue; }
            let rel = match path.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push(WalkedFile {
                relative_path: format!(
                    "ext:tf-registry:{}/{}",
                    dep.module_path.replace('/', "_"),
                    rel
                ),
                absolute_path: path,
                language: "hcl",
            });
        }
    }
}

// =============================================================================
// Symbol-location index (demand-driven)
// =============================================================================

fn build_tf_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    let mut index = SymbolLocationIndex::new();
    for dep in dep_roots {
        if dep.module_path == "tf-providers-bundled" { continue; }
        for wf in walk_tf_module_dir(dep) {
            let Ok(src) = std::fs::read_to_string(&wf.absolute_path) else { continue };
            for sym_name in scan_tf_top_level_resources(&src) {
                index.insert(dep.module_path.clone(), sym_name, wf.absolute_path.clone());
            }
        }
    }
    index
}

/// Line-based scan of a `.tf` file for top-level `resource` and `data` block
/// declarations. Returns the qualified Terraform name:
///   `resource "aws_vpc" "this"` → `"aws_vpc.this"`
///   `data "aws_ami" "latest"`   → `"data.aws_ami.latest"`
pub(crate) fn scan_tf_top_level_resources(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        // resource "type" "name" {
        if let Some(rest) = trimmed.strip_prefix("resource ") {
            if let Some((rtype, rname)) = parse_two_labels(rest) {
                out.push(format!("{rtype}.{rname}"));
            }
        }
        // data "type" "name" {
        else if let Some(rest) = trimmed.strip_prefix("data ") {
            if let Some((dtype, dname)) = parse_two_labels(rest) {
                out.push(format!("data.{dtype}.{dname}"));
            }
        }
        // module "name" {
        else if let Some(rest) = trimmed.strip_prefix("module ") {
            if let Some(name) = parse_one_label(rest) {
                out.push(format!("module.{name}"));
            }
        }
        // output "name" {
        else if let Some(rest) = trimmed.strip_prefix("output ") {
            if let Some(name) = parse_one_label(rest) {
                out.push(format!("output.{name}"));
            }
        }
    }
    out
}

fn parse_two_labels(s: &str) -> Option<(&str, &str)> {
    let mut parts = s.trim().splitn(3, '"');
    let _ = parts.next()?; // empty before first quote
    let first = parts.next()?.trim();
    let rest = parts.next()?;
    let mut inner = rest.trim().splitn(3, '"');
    let _ = inner.next()?; // whitespace between labels
    let second = inner.next()?.trim();
    if first.is_empty() || second.is_empty() { return None; }
    Some((first, second))
}

fn parse_one_label(s: &str) -> Option<&str> {
    let mut parts = s.trim().splitn(3, '"');
    let _ = parts.next()?;
    let name = parts.next()?.trim();
    if name.is_empty() { return None; }
    Some(name)
}

// =============================================================================
// Provider + module source parsing (regex-free, line-based)
// =============================================================================

/// Extract `source = "namespace/name"` values from `required_providers` blocks
/// in the given HCL content.
pub(crate) fn extract_required_providers(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_req_providers = false;
    let mut depth: i32 = 0;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.contains("required_providers") && trimmed.contains('{') {
            in_req_providers = true;
            depth = 1;
            continue;
        }
        if !in_req_providers { continue; }

        for ch in trimmed.chars() {
            if ch == '{' { depth += 1; }
            else if ch == '}' { depth -= 1; }
        }
        if depth <= 0 {
            in_req_providers = false;
            depth = 0;
            continue;
        }
        // Inside the block: look for `source = "hashicorp/aws"`
        if let Some(src) = extract_source_value(trimmed) {
            out.push(src.to_string());
        }
    }
    out
}

/// Extract `source` attribute values from `module "x" { source = "..." }` blocks.
pub(crate) fn extract_module_sources(content: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_module = false;
    let mut depth: i32 = 0;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("module ") && trimmed.contains('{') {
            in_module = true;
            depth = 1;
            continue;
        }
        if !in_module { continue; }

        for ch in trimmed.chars() {
            if ch == '{' { depth += 1; }
            else if ch == '}' { depth -= 1; }
        }
        if depth <= 0 {
            in_module = false;
            depth = 0;
            continue;
        }
        if let Some(src) = extract_source_value(trimmed) {
            // Skip local paths (./ or ../)
            if !src.starts_with('.') && !src.starts_with('/') {
                out.push(src.to_string());
            }
        }
    }
    out
}

/// Extract the string value from `source = "..."` or `source  =  "..."`.
fn extract_source_value(line: &str) -> Option<&str> {
    let after_source = line.strip_prefix("source")?.trim_start();
    let after_eq = after_source.strip_prefix('=')?.trim_start();
    let inner = after_eq.strip_prefix('"')?.split('"').next()?;
    if inner.is_empty() { return None; }
    Some(inner)
}

// =============================================================================
// Bundled synthetic provider symbols  (MVP)
// =============================================================================
//
// Providers ship as Go binaries — there is no Terraform-readable source in the
// project's `.terraform/providers/` directory that tree-sitter can parse.
//
// For MVP we bundle the ~30 most-used resource types for the top 3 providers.
// This is intentionally a hard-coded list. A future session can extend this by
// reading `.terraform/providers/…/terraform-provider-*/schema.json` when
// present, or by querying the Terraform Registry JSON API.
//
// Each resource type becomes a `SymbolKind::Class` in a virtual file at
// `ext:tf-registry:<provider>/resources.tf`. The qualified_name matches the
// pattern the HCL extractor emits for `resource "aws_vpc" "this"` references:
// `aws_vpc` (the type name without instance label).
//
// MVP-BUNDLED: extend this list or replace with dynamic schema loading.

struct ProviderResource {
    provider: &'static str, // short name: "aws", "google", "azurerm"
    resource: &'static str, // resource type: "aws_vpc"
}

/// MVP-BUNDLED top resources for aws, google, azurerm.
/// This list intentionally covers only the most commonly encountered resource
/// types to keep the synthetic symbol count small and boot time fast.
/// A schema-driven extension point is left as a TODO comment above.
const BUNDLED_RESOURCES: &[ProviderResource] = &[
    // ---- AWS (hashicorp/aws) ----
    ProviderResource { provider: "aws", resource: "aws_vpc" },
    ProviderResource { provider: "aws", resource: "aws_subnet" },
    ProviderResource { provider: "aws", resource: "aws_internet_gateway" },
    ProviderResource { provider: "aws", resource: "aws_route_table" },
    ProviderResource { provider: "aws", resource: "aws_route_table_association" },
    ProviderResource { provider: "aws", resource: "aws_security_group" },
    ProviderResource { provider: "aws", resource: "aws_instance" },
    ProviderResource { provider: "aws", resource: "aws_s3_bucket" },
    ProviderResource { provider: "aws", resource: "aws_s3_bucket_policy" },
    ProviderResource { provider: "aws", resource: "aws_iam_role" },
    ProviderResource { provider: "aws", resource: "aws_iam_policy" },
    ProviderResource { provider: "aws", resource: "aws_iam_role_policy_attachment" },
    ProviderResource { provider: "aws", resource: "aws_lambda_function" },
    ProviderResource { provider: "aws", resource: "aws_cloudwatch_log_group" },
    ProviderResource { provider: "aws", resource: "aws_db_instance" },
    ProviderResource { provider: "aws", resource: "aws_eks_cluster" },
    ProviderResource { provider: "aws", resource: "aws_eks_node_group" },
    ProviderResource { provider: "aws", resource: "aws_vpc_ipv4_cidr_block_association" },
    ProviderResource { provider: "aws", resource: "aws_nat_gateway" },
    ProviderResource { provider: "aws", resource: "aws_eip" },
    // ---- Google Cloud (hashicorp/google) ----
    ProviderResource { provider: "google", resource: "google_compute_instance" },
    ProviderResource { provider: "google", resource: "google_compute_network" },
    ProviderResource { provider: "google", resource: "google_compute_subnetwork" },
    ProviderResource { provider: "google", resource: "google_storage_bucket" },
    ProviderResource { provider: "google", resource: "google_container_cluster" },
    ProviderResource { provider: "google", resource: "google_container_node_pool" },
    ProviderResource { provider: "google", resource: "google_project_iam_member" },
    ProviderResource { provider: "google", resource: "google_sql_database_instance" },
    // ---- Azure (hashicorp/azurerm) ----
    ProviderResource { provider: "azurerm", resource: "azurerm_resource_group" },
    ProviderResource { provider: "azurerm", resource: "azurerm_virtual_network" },
    ProviderResource { provider: "azurerm", resource: "azurerm_subnet" },
    ProviderResource { provider: "azurerm", resource: "azurerm_network_security_group" },
    ProviderResource { provider: "azurerm", resource: "azurerm_linux_virtual_machine" },
    ProviderResource { provider: "azurerm", resource: "azurerm_storage_account" },
    ProviderResource { provider: "azurerm", resource: "azurerm_kubernetes_cluster" },
];

fn synthesize_bundled_providers() -> Vec<ParsedFile> {
    // Group by provider so we emit one ParsedFile per provider.
    use std::collections::HashMap;
    let mut by_provider: HashMap<&'static str, Vec<&ProviderResource>> = HashMap::new();
    for res in BUNDLED_RESOURCES {
        by_provider.entry(res.provider).or_default().push(res);
    }

    let mut out = Vec::new();
    for (provider, resources) in &by_provider {
        let virtual_path = format!("ext:tf-registry:{provider}/resources.tf");
        let mut symbols: Vec<ExtractedSymbol> = Vec::new();

        // One Class symbol per resource type. The name is the Terraform resource
        // type (e.g. "aws_vpc"); qualified_name matches what the resolver sees.
        for res in resources {
            symbols.push(ExtractedSymbol {
                name: res.resource.to_string(),
                qualified_name: res.resource.to_string(),
                kind: SymbolKind::Class,
                visibility: Some(Visibility::Public),
                start_line: 0,
                end_line: 0,
                start_col: 0,
                end_col: 0,
                signature: Some(format!("resource \"{}\" {{}}", res.resource)),
                doc_comment: None,
                scope_path: Some(provider.to_string()),
                parent_index: None,
            });
        }

        out.push(build_parsed_file(virtual_path, symbols));
    }
    out
}

fn build_parsed_file(virtual_path: String, symbols: Vec<ExtractedSymbol>) -> ParsedFile {
    let content_hash = format!("tf-bundled-{:x}", symbols.len());
    ParsedFile {
        path: virtual_path,
        language: "hcl".to_string(),
        content_hash,
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

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
        // Local paths must be excluded
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
        let files = synthesize_bundled_providers();
        // Must have one file per provider (aws, google, azurerm)
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(
            paths.iter().any(|p| p.contains(":aws/")),
            "aws provider missing: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.contains(":google/")),
            "google provider missing: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.contains(":azurerm/")),
            "azurerm provider missing: {paths:?}"
        );
        // Spot-check a few known resources exist as symbols
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
        for pf in synthesize_bundled_providers() {
            assert!(!pf.symbols.is_empty(), "provider file {} has no symbols", pf.path);
            for sym in &pf.symbols {
                assert!(!sym.name.is_empty(), "empty symbol name in {}", pf.path);
                assert_eq!(sym.kind, SymbolKind::Class, "expected Class kind for {}", sym.name);
            }
        }
    }

    #[test]
    fn parse_two_labels_handles_aligned_quotes() {
        assert_eq!(
            parse_two_labels(r#""aws_vpc" "this" {"#),
            Some(("aws_vpc", "this"))
        );
    }

    #[test]
    fn extract_source_value_handles_extra_spaces() {
        assert_eq!(extract_source_value(r#"source  =  "hashicorp/aws""#), Some("hashicorp/aws"));
        assert_eq!(extract_source_value(r#"source = """#), None);
        assert_eq!(extract_source_value(r#"version = "1.0""#), None);
    }
}
