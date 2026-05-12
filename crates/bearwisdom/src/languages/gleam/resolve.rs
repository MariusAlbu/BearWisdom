// =============================================================================
// languages/gleam/resolve.rs — Gleam resolution rules
//
// Gleam uses a simple module system:
//
//   import gleam/list           → Imports, target_name = "list",  module = "gleam/list"
//   import myapp/utils          → Imports, target_name = "utils", module = "myapp/utils"
//   list.map(xs, f)             → Calls,   target_name = "map",   module = None
//   local_function()            → Calls,   target_name = "local_function", module = None
//
// The extractor strips the module qualifier from call sites (field_access nodes
// emit only the function name). So "list.map" becomes target_name = "map".
//
// Resolution strategy:
//   1. Same-file: functions defined in the same file are always in scope.
//   2. Import-based: for each imported module, try `{last_segment}.{target}`
//      as a qualified name, then a bare name lookup within the module.
//   3. Name-only fallback: global by_name lookup with lower confidence.
// =============================================================================

use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct GleamResolver;

impl LanguageResolver for GleamResolver {
    fn language_ids(&self) -> &[&str] {
        &["gleam"]
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
            let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias: None,
                // Gleam module imports bring qualified access into scope.
                // Mark as wildcard so the import walk can classify unresolved
                // bare names from external modules.
                is_wildcard: true,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "gleam".to_string(),
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

        // Skip import declarations — they declare scope, not symbol references.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Skip Gleam built-in operators emitted from binary_expression.
        if is_gleam_operator(target) {
            return None;
        }

        // Language-specific: import-based resolution with module alias lookup.
        // Gleam qualified names are stored as `module.function` in the index.
        for import in &file_ctx.imports {
            let Some(full_path) = &import.module_path else {
                continue;
            };

            // The local alias is the last path segment (e.g., "list" for "gleam/list").
            let module_alias = import
                .alias
                .as_deref()
                .unwrap_or_else(|| full_path.rsplit('/').next().unwrap_or(full_path.as_str()));

            // Try qualified name: {module}.{target}
            let candidate = format!("{module_alias}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "gleam_import_qualified",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }

            // Try bare name within the imported module's file.
            for sym in lookup.in_file(full_path) {
                if sym.name == *target {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "gleam_import_file",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }

        engine::resolve_common("gleam", file_ctx, ref_ctx, lookup, |_, _| true)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Gleam stdlib modules start with "gleam/" — mark them external before
        // the common handler so the specific namespace is preserved.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let path = ref_ctx
                .extracted_ref
                .module
                .as_deref()
                .unwrap_or(target.as_str());
            if path.starts_with("gleam/") {
                return Some(path.to_string());
            }
        }

        if is_gleam_operator(target) {
            return Some("builtin".to_string());
        }

        // Stdlib function names classify via the engine's keywords() set
        // populated from gleam/mod.rs::keywords(); gleam_stdlib + hex
        // walkers emit real symbols for declared deps.
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, |_| false)
    }
}

/// Gleam binary operators emitted by the extractor as Calls refs.
/// These are language-level operators — not project symbols.
fn is_gleam_operator(name: &str) -> bool {
    matches!(
        name,
        "+" | "-" | "*" | "/" | "%" | "==" | "!=" | "<" | "<=" | ">" | ">="
            | "&&" | "||" | "!" | "|>" | "<>" | "+." | "-." | "*." | "/."
            | "==." | "!=." | "<." | "<=." | ">." | ">=."
    )
}
