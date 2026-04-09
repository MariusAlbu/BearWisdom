// =============================================================================
// languages/hcl/resolve.rs — HCL / Terraform resolution rules
//
// HCL (HashiCorp Configuration Language) / Terraform reference patterns:
//
//   var.name          → reference to a variable block
//   local.name        → reference to a locals block entry
//   data.type.name    → reference to a data source
//   module.name       → reference to a module block
//   resource_type.name.attr  → cross-resource reference
//
// The extractor emits variable_expr / get_attr / function_call refs. The
// target_name is typically the variable name or function name.
//
// Resolution strategy:
//   1. Same-file: all HCL blocks (variable, local, module, resource) are
//      visible within the same file (Terraform merges all .tf in a directory,
//      but within a file is highest confidence).
//   2. Global lookup: Terraform merges all .tf files in a module directory,
//      so cross-file resolution at lower confidence.
//   3. Provider resource types and built-in functions are external.
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct HclResolver;

impl LanguageResolver for HclResolver {
    fn language_ids(&self) -> &[&str] {
        &["hcl", "terraform"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        // HCL has no explicit import statements. All .tf files in a directory
        // form a single module — scope is directory-wide, handled by global
        // lookup with directory filtering.
        FileContext {
            file_path: file.path.clone(),
            language: "hcl".to_string(),
            imports: Vec::new(),
            file_namespace: None,
        }
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        // Import declarations (module source references) are external.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Built-in Terraform/HCL functions are never in the project index.
        if builtins::is_hcl_builtin(target) {
            return None;
        }

        // Terraform meta-references — no project symbol.
        if is_terraform_meta_ref(target) {
            return None;
        }

        // ----------------------------------------------------------------
        // Step 1: direct qualified-name lookup.
        //
        // module.*, data.*.*, and resource_type.* refs are emitted with a
        // target_name that exactly matches the symbol's qualified_name in
        // the index.  Try this first before any prefix-stripping.
        // Terraform merges all .tf files in a directory, so check the
        // whole project index (not just the same file) at full confidence.
        // ----------------------------------------------------------------
        if let Some(sym) = lookup.by_qualified_name(target) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "hcl_qname_direct",
            });
        }

        // ----------------------------------------------------------------
        // Step 2: prefix-stripped Variable lookup for var.* and local.*.
        //
        // "var.region" → strip "var." → look for Variable named "region"
        // "local.env"  → strip "local." → look for Variable named "env"
        // Terraform modules share scope across all .tf in a directory, so
        // check same-file first (highest confidence), then project-wide.
        // ----------------------------------------------------------------
        let bare = strip_hcl_prefix(target);
        if bare != target.as_str() {
            // Same-file match first.
            for sym in lookup.in_file(&file_ctx.file_path) {
                if sym.name == bare {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "hcl_same_file_bare",
                    });
                }
            }
            // Cross-file match (Terraform directory scope).
            for sym in lookup.by_name(bare) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.9,
                    strategy: "hcl_cross_file_bare",
                });
            }
        }

        engine::resolve_common("hcl", file_ctx, ref_ctx, lookup, |_, _| true)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        if builtins::is_hcl_builtin(target) {
            return Some("hcl".to_string());
        }

        // Import edges (module source attribute) are always external.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return Some("terraform".to_string());
        }

        // Terraform meta-references (each.*, count.*, self.*, path.*, terraform.*).
        if is_terraform_meta_ref(target) {
            return Some("terraform".to_string());
        }

        // data.* that refer to provider data sources (not locally-defined data blocks).
        if target.starts_with("data.") {
            let parts: Vec<&str> = target.splitn(3, '.').collect();
            if parts.len() >= 2 && is_provider_resource_type(parts[1]) {
                return Some("terraform".to_string());
            }
        }

        // Resource-type references where the prefix is a known provider type
        // (e.g. "aws_instance.web" — provider already handled the symbol, but if
        // the resource wasn't indexed this classifies the ref as external).
        if let Some(dot) = target.find('.') {
            let prefix = &target[..dot];
            if is_provider_resource_type(prefix) {
                return Some("terraform".to_string());
            }
        }

        engine::infer_external_common(file_ctx, ref_ctx, builtins::is_hcl_builtin)
    }
}

/// Strip the `var.` or `local.` prefix to get the bare Variable name.
///
/// Only applies to the two prefixes whose symbols are stored without prefix
/// in the index (`variable "region"` → symbol name `"region"`).
/// `module.*` and `data.*` refs already match qualified_name directly.
fn strip_hcl_prefix(name: &str) -> &str {
    for prefix in ["var.", "local."] {
        if let Some(rest) = name.strip_prefix(prefix) {
            // Strip any trailing attribute access (e.g. "var.obj.field" → "obj").
            return rest.splitn(2, '.').next().unwrap_or(rest);
        }
    }
    name
}

/// Returns true for Terraform meta-references that never resolve to a project
/// symbol: `each.*`, `count.*`, `self.*`, `path.*`, `terraform.*`.
fn is_terraform_meta_ref(name: &str) -> bool {
    matches!(
        name.splitn(2, '.').next().unwrap_or(name),
        "each" | "count" | "self" | "path" | "terraform"
    )
}

/// Check if a name looks like a Terraform provider resource type.
/// Provider resource types follow a `provider_resourcetype` pattern.
fn is_provider_resource_type(name: &str) -> bool {
    // Provider resource types contain an underscore and match patterns like
    // "aws_instance", "azurerm_resource_group", "google_compute_instance".
    name.contains('_')
        && (name.starts_with("aws_")
            || name.starts_with("azurerm_")
            || name.starts_with("google_")
            || name.starts_with("kubernetes_")
            || name.starts_with("helm_")
            || name.starts_with("null_")
            || name.starts_with("random_")
            || name.starts_with("local_")
            || name.starts_with("tls_")
            || name.starts_with("vault_")
            || name.starts_with("consul_")
            || name.starts_with("nomad_"))
}

