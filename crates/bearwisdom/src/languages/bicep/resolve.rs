// =============================================================================
// languages/bicep/resolve.rs — Bicep resolution rules
//
// Bicep is an IaC DSL for Azure. References in Bicep files are:
//
//   module myMod './child.bicep' = { ... }     → module declaration, file ref
//   resource rg 'Microsoft.Resources/resourceGroups@2021-04-01' = { ... }
//     → resource declaration, type string is the Azure resource type (external)
//   import { Foo } from 'br:...'               → import statement
//   myVar                                      → variable reference (Calls/TypeRef)
//
// Resolution strategy:
//   1. Import-based: collect `import`/`using` statement refs; look up the
//      imported name directly.
//   2. Same-file: all top-level declarations (module, resource, param, var,
//      output) are in scope for the whole file.
//   3. Global name fallback with lower confidence.
//
// External namespaces:
//   - Azure resource type strings (e.g. "Microsoft.Compute/virtualMachines")
//     are classified as `"azure"`.
//   - Bicep built-in functions are classified as `"bicep"`.
// =============================================================================

use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct BicepResolver;

impl LanguageResolver for BicepResolver {
    fn language_ids(&self) -> &[&str] {
        &["bicep"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: r.module.clone().or_else(|| Some(r.target_name.clone())),
                alias: None,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "bicep".to_string(),
            imports,
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

        // Import declarations don't need resolving to a symbol.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Azure resource type strings are external — skip resolution.
        if is_azure_resource_type(target) {
            return None;
        }

        engine::resolve_common("bicep", file_ctx, ref_ctx, lookup, bicep_kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Azure resource type strings take priority.
        if is_azure_resource_type(target) {
            return Some("azure".to_string());
        }

        engine::infer_external_common(file_ctx, ref_ctx, is_bicep_builtin)
    }
}

/// Edge kind / symbol kind compatibility for Bicep.
fn bicep_kind_compatible(edge_kind: crate::types::EdgeKind, sym_kind: &str) -> bool {
    use crate::types::EdgeKind;
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "variable" | "function"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// Returns true for Azure resource provider type strings.
/// These always contain a "/" (e.g. "Microsoft.Compute/virtualMachines@2023-03-01").
fn is_azure_resource_type(name: &str) -> bool {
    // Azure resource type strings have the form "Provider/type@api-version" or
    // "Provider/type". They always contain a "/" and typically start with a
    // known Azure provider prefix.
    if !name.contains('/') {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    lower.starts_with("microsoft.")
        || lower.starts_with("azure.")
        || lower.starts_with("br:")
        || lower.starts_with("br/")
}

/// Bicep built-in functions that are never in the project symbol index.
fn is_bicep_builtin(name: &str) -> bool {
    matches!(
        name,
        // Resource functions
        "resourceId" | "subscriptionResourceId" | "tenantResourceId"
            | "extensionResourceId" | "resourceGroup" | "subscription"
            | "tenant" | "managementGroup" | "deployment"
            // String functions
            | "concat" | "format" | "base64" | "base64ToString" | "uriComponent"
            | "uri" | "toLower" | "toUpper" | "trim" | "split" | "join"
            | "replace" | "startsWith" | "endsWith" | "contains" | "length"
            | "substring" | "indexOf" | "lastIndexOf" | "empty" | "string"
            | "int" | "bool" | "json" | "null"
            // Array / object functions
            | "array" | "createArray" | "union" | "intersection" | "first"
            | "last" | "min" | "max" | "range" | "flatten" | "filter"
            | "map" | "sort" | "reduce" | "toObject" | "items"
            | "objectKeys" | "values"
            // Numeric
            | "add" | "sub" | "mul" | "div" | "mod"
            // Type check
            | "getType" | "isObject" | "isArray" | "isString" | "isInt" | "isBool"
            // Other
            | "newGuid" | "utcNow" | "dateTimeAdd" | "dateTimeToEpoch"
            | "environment" | "loadTextContent" | "loadJsonContent"
            | "loadYamlContent" | "loadFileAsBase64" | "readEnvironmentVariable"
            | "sys" | "az" | "any"
    )
}
