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

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile, SymbolKind};

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

        if let Some(res) =
            engine::resolve_common("bicep", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
        {
            return Some(res);
        }

        // Bicep doesn't have user-authored imports for built-in functions,
        // decorators, or resource API methods — those names live in the
        // synthetic ParsedFile emitted by the `bicep-runtime` ecosystem
        // when an Azure/bicep source clone is discovered on disk. The
        // shared `resolve_common` path requires either an import or a
        // same-file declaration to bind a name; neither applies here.
        // Fall back to a global by-name lookup that ONLY matches symbols
        // qualified under `bicep.*` (i.e. the runtime grammar synthetic),
        // so we don't drift into binding bare names against unrelated
        // symbols from other languages in the index.
        resolve_against_bicep_runtime(target, edge_kind, lookup)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        // Azure resource type strings take priority.
        if is_azure_resource_type(target) {
            return Some("azure".to_string());
        }

        // Child-resource shorthand: inside a parent `resource` block a nested
        // `resource childAlias 'subnets' existing = { ... }` uses a bare
        // single-segment type name that resolves against the parent's type
        // path at deploy time (→ `Microsoft.Network/virtualNetworks/subnets`).
        // The bicep extractor only emits TypeRef refs for resource-declaration
        // type strings, so any bare-name TypeRef here is a child shorthand.
        if edge_kind == EdgeKind::TypeRef && is_child_resource_shorthand(target) {
            return Some("azure".to_string());
        }

        // No predicate-based builtin classification: builtin names come from
        // the `bicep-runtime` ecosystem walker via the symbol index. If a
        // ref still doesn't resolve here, it's genuinely unresolved (the
        // user has no local Azure/bicep source clone for the walker to
        // find).
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, |_| false)
    }
}

/// Match a bare-name target against the bicep-runtime ecosystem's
/// synthetic ParsedFile. The synthetic file lives at
/// `ext:bicep-runtime:namespace.bicep` with symbols qualified under
/// `bicep.builtins.*`, `bicep.decorators.*`, `bicep.namespace.*`. Matching
/// is also case-insensitive on the bare segment so real-world Bicep's
/// case-insensitive function names (`listkeys` vs `listKeys`, `if` vs `IF`)
/// resolve correctly.
fn resolve_against_bicep_runtime(
    target: &str,
    edge_kind: EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    if target.is_empty() {
        return None;
    }
    // Strip a leading `sys.` / `az.` namespace alias, plus a trailing
    // `<resourceVar>.<apiMethod>` shape — both bind to the bare method/fn
    // name in the runtime grammar.
    let bare = target
        .strip_prefix("sys.")
        .or_else(|| target.strip_prefix("az."))
        .or_else(|| target.rsplit_once('.').map(|(_, t)| t))
        .unwrap_or(target);

    if !bare.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }

    let lower = bare.to_ascii_lowercase();
    for sym in lookup.by_name(bare).into_iter().chain(lookup.by_name(&lower)) {
        if !sym.qualified_name.starts_with("bicep.") {
            continue;
        }
        if !predicates::kind_compatible(edge_kind, &sym.kind) {
            continue;
        }
        // Ignore the original SymbolKind variable; we only care about
        // matching, then return a Resolution.
        let _ = SymbolKind::Function;
        return Some(Resolution {
            target_symbol_id: sym.id,
            confidence: 0.9,
            strategy: "bicep_runtime_grammar",
            resolved_yield_type: None,
        });
    }
    None
}

/// Returns true for Azure resource provider type strings.
/// These always contain a "/" (e.g. "Microsoft.Compute/virtualMachines@2023-03-01").
///
/// The shape is `<Namespace>[.<SubNamespace>]/<typeName>[/<subType>...][@<apiVersion>]`.
/// We accept any `<dotted-namespace>/<typeName>` payload — not just `Microsoft.*`
/// — so test fixtures (`My.Rp/parentType@2020-12-01`, `Mock.Rp/...`) and
/// third-party providers route as external rather than dragging the resolution
/// rate down. ACR registry refs (`br:` / `br/`) keep their own arms.
fn is_azure_resource_type(name: &str) -> bool {
    // Strip wrapping single quotes — the bicep extractor sometimes emits
    // quoted resource type strings (`'Microsoft.Foo/foos@2020-02-02-alpha'`)
    // as Calls when the type-string appears in an unexpected position.
    let stripped = name.trim_matches('\'');
    if !stripped.contains('/') {
        return false;
    }
    let lower = stripped.to_ascii_lowercase();
    // Registry aliases: ACR (`br:`/`br/`) and `az:` for the public registry.
    if lower.starts_with("br:") || lower.starts_with("br/") || lower.starts_with("az:") {
        return true;
    }
    // Generic shape: `<head>/<rest>` where head is alphanumeric with optional
    // dots (`Microsoft.Compute`, `My.Rp`, `apps`, `core`) and string-template
    // interpolation tokens (`${provider}`). The interpolation chars don't
    // appear in user-defined identifiers so they're a strong signal this is
    // a deploy-time string, not a Bicep symbol reference.
    let head = stripped.split('/').next().unwrap_or("");
    if head.is_empty() {
        return false;
    }
    head.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '$' | '{' | '}'))
}

/// Returns true for the bare type-name shorthand used inside nested
/// `resource` declarations (`'subnets'`, `'ruleCollectionGroups'`, `'A'`).
/// Valid shorthand forms are a single alphanumeric segment, optionally
/// with an `@api-version` suffix. Two casing conventions are accepted:
///   * fully-lowercase camelCase: `subnets`, `ruleCollectionGroups`
///   * fully-uppercase DNS record types: `A`, `AAAA`, `CNAME`, `MX`, `TXT`
/// PascalCase (`MyOwnResource`) is intentionally rejected — those are
/// user-defined symbol references, not child-resource shorthand.
fn is_child_resource_shorthand(name: &str) -> bool {
    if name.is_empty() || name.contains('/') {
        return false;
    }
    let base = name.split('@').next().unwrap_or(name);
    if base.is_empty() {
        return false;
    }
    if !base.chars().all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    // Accept either all-lowercase-start (camelCase: subnets, ruleCollectionGroups)
    // or all-uppercase (DNS records: A, AAAA, CNAME). PascalCase looks like
    // a user-defined symbol and is rejected.
    let starts_lower = base.chars().next().map(|c| c.is_ascii_lowercase()).unwrap_or(false);
    let all_upper = base.chars().all(|c| !c.is_ascii_lowercase());
    starts_lower || all_upper
}

