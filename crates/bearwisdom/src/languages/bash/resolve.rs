// =============================================================================
// bash/resolve.rs — Bash resolution rules
//
// Scope rules for Bash/Shell:
//
//   1. Scope chain walk: innermost function → outermost.
//   2. Same-file resolution: all top-level functions are visible within
//      the file.
//   3. By-name lookup: for sourced files, functions may be defined
//      in another file brought in via `source` or `.`.
//
// Bash import model:
//   `source script.sh`  → target_name = "script.sh"
//   `. script.sh`       → target_name = "script.sh"
//
// The extractor emits EdgeKind::Imports with target_name = the sourced path.
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Bash language resolver.
pub struct BashResolver;

impl LanguageResolver for BashResolver {
    fn language_ids(&self) -> &[&str] {
        &["shell"]
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
                module_path: Some(r.target_name.clone()),
                alias: None,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "shell".to_string(),
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

        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Bash builtins are never in the index.
        if builtins::is_bash_builtin(target) {
            return None;
        }

        // Step 1: Scope chain walk.
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "bash_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == *target && builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "bash_same_file",
                });
            }
        }

        // Step 3: By-name lookup across the project (sourced files).
        for sym in lookup.by_name(target) {
            if builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "bash_by_name",
                });
            }
        }

        None
    }
}
