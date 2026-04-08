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
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
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

        engine::resolve_common("erlang", file_ctx, ref_ctx, lookup, builtins::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, builtins::is_erlang_builtin)
    }
}
