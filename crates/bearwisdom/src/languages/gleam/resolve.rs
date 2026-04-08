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
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
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
                is_wildcard: false,
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

        // Step 1: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == *target {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "gleam_same_file",
                });
            }
        }

        // Step 2: Import-based resolution.
        // For each import, try `{module_name}.{target}` (Gleam qualified names
        // are stored as `module.function` in the symbol index).
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
                });
            }

            // Try bare name within the imported module's file.
            for sym in lookup.in_file(full_path) {
                if sym.name == *target {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "gleam_import_file",
                    });
                }
            }
        }

        // Step 3: Global name fallback.
        let candidates = lookup.by_name(target);
        if let Some(sym) = candidates.into_iter().next() {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.8,
                strategy: "gleam_global_name",
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

        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let path = ref_ctx
                .extracted_ref
                .module
                .as_deref()
                .unwrap_or(target.as_str());
            // Gleam stdlib modules start with "gleam/" — mark them external.
            if path.starts_with("gleam/") {
                return Some(path.to_string());
            }
        }

        if is_gleam_operator(target) {
            return Some("builtin".to_string());
        }

        None
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
