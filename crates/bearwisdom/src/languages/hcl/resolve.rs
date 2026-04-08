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

use crate::indexer::resolve::engine::{
    FileContext, LanguageResolver, RefContext, Resolution, SymbolLookup,
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
        if is_hcl_builtin(target) {
            return None;
        }

        // Strip common prefixes like "var.", "local.", "module." to get the
        // bare name for lookup.
        let bare = strip_hcl_prefix(target);

        // Step 1: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == bare || sym.name == *target {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "hcl_same_file",
                });
            }
        }

        // Step 2: Global lookup (cross-file within same Terraform module dir).
        for sym in lookup.by_name(bare) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.85,
                strategy: "hcl_global",
            });
        }

        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        if is_hcl_builtin(target) {
            return Some("hcl".to_string());
        }

        // Provider resource type references (e.g. "aws_instance", "azurerm_resource_group")
        // and module source registry references are external.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return Some("terraform".to_string());
        }

        // References starting with "data." that refer to provider data sources.
        if target.starts_with("data.") {
            let parts: Vec<&str> = target.splitn(3, '.').collect();
            if parts.len() >= 2 && is_provider_resource_type(parts[1]) {
                return Some("terraform".to_string());
            }
        }

        None
    }
}

/// Strip common HCL reference prefixes to get the bare name.
fn strip_hcl_prefix(name: &str) -> &str {
    // "var.foo" → "foo", "local.foo" → "foo", "module.foo" → "foo"
    // "data.type.foo" → handled separately, just strip "data."
    for prefix in ["var.", "local.", "module.", "data."] {
        if let Some(rest) = name.strip_prefix(prefix) {
            // For "data.type.name", return the next segment.
            return rest.splitn(2, '.').next().unwrap_or(rest);
        }
    }
    name
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

/// Terraform / HCL built-in functions.
fn is_hcl_builtin(name: &str) -> bool {
    matches!(
        name,
        // Numeric
        "abs" | "ceil" | "floor" | "log" | "max" | "min" | "parseint" | "pow" | "signum"
            // String
            | "chomp" | "endswith" | "format" | "formatlist" | "indent" | "join"
            | "lower" | "ltrim" | "regex" | "regexall" | "replace" | "rtrim"
            | "split" | "startswith" | "strcontains" | "strrev" | "substr"
            | "templatestring" | "title" | "trim" | "trimprefix" | "trimsuffix"
            | "trimspace" | "upper"
            // Collection
            | "alltrue" | "anytrue" | "chunklist" | "coalesce" | "coalescelist"
            | "compact" | "concat" | "contains" | "distinct" | "element" | "flatten"
            | "index" | "keys" | "length" | "list" | "lookup" | "map" | "matchkeys"
            | "merge" | "one" | "range" | "reverse" | "setintersection" | "setproduct"
            | "setsubtract" | "setunion" | "slice" | "sort" | "sum" | "tolist"
            | "tomap" | "toset" | "transpose" | "values" | "zipmap"
            // Encoding
            | "base64decode" | "base64encode" | "base64gzip" | "csvdecode" | "jsondecode"
            | "jsonencode" | "textdecodebase64" | "textencodebase64" | "urlencode"
            | "yamldecode" | "yamlencode"
            // Filesystem
            | "abspath" | "dirname" | "pathexpand" | "basename" | "file"
            | "fileexists" | "fileset" | "filebase64" | "filebase64sha256"
            | "filebase64sha512" | "filemd5" | "filesha1" | "filesha256" | "filesha512"
            | "templatefile"
            // Date/time
            | "formatdate" | "plantimestamp" | "timeadd" | "timecmp" | "timestamp"
            // Hash / crypto
            | "base64sha256" | "base64sha512" | "bcrypt" | "md5" | "rsadecrypt"
            | "sha1" | "sha256" | "sha512" | "uuid" | "uuidv5"
            // IP / networking
            | "cidrhost" | "cidrnetmask" | "cidrsubnet" | "cidrsubnets"
            // Type conversion
            | "can" | "issensitive" | "nonsensitive" | "sensitive" | "tobool"
            | "tonumber" | "tostring" | "try" | "type"
            // Object
            | "object" | "tuple"
            // HCL meta-functions
            | "each" | "count" | "path" | "self" | "terraform"
    )
}
