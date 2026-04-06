// =============================================================================
// erlang/resolve.rs — Erlang resolution rules
//
// Scope rules for Erlang:
//
//   1. Scope chain walk: innermost function → module level.
//   2. Same-file resolution: all top-level functions in the module are visible.
//   3. Import-based resolution: `-import(Module, [Fun/Arity]).` and
//      `-include("header.hrl").` bring external symbols into scope.
//
// Erlang import model:
//   `-module(mod_name).`          → declares the module name
//   `-import(Module, [Fun/Arity]).` → imports specific functions from a module
//   `-include("header.hrl").`       → textual include (local header)
//   Module:function()               → remote call (not an import)
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Erlang language resolver.
pub struct ErlangResolver;

impl LanguageResolver for ErlangResolver {
    fn language_ids(&self) -> &[&str] {
        &["erlang"]
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
            // target_name is the module name or include path.
            // module, if present, holds the original module for `Module:Fun` remote calls.
            let module_path = r.module.clone().or_else(|| Some(r.target_name.clone()));
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path,
                alias: None,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "erlang".to_string(),
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

        // Erlang BIFs and stdlib modules are not in the index.
        if builtins::is_erlang_builtin(target) {
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
                        strategy: "erlang_scope_chain",
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
                    strategy: "erlang_same_file",
                });
            }
        }

        // Step 3: Simple name lookup across the project.
        for sym in lookup.by_name(target) {
            if builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "erlang_by_name",
                });
            }
        }

        None
    }
}
