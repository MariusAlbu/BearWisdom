// =============================================================================
// prolog/resolve.rs — Prolog resolution rules
//
// Prolog module system (SWI-Prolog / ISO):
//   - `:- module(Name, Exports).` — declares the module name and exported predicates.
//   - `:- use_module(library(lists)).` — imports a standard library module.
//   - `:- use_module(Module).` — imports all public predicates from Module.
//   - `:- use_module(Module, [pred/arity, ...]).` — selective import.
//   - Unqualified predicates resolve to the current module first, then imported
//     modules, then the global `user` module.
//
// Resolution strategy:
//   1. Scope chain walk (not typical in Prolog, but handles meta-predicates that
//      introduce local scope, e.g., lambda or DCG rules).
//   2. Same-file resolution (predicates defined in the same source file).
//   3. Project-wide name lookup (exported predicates from other modules).
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Prolog language resolver.
pub struct PrologResolver;

impl LanguageResolver for PrologResolver {
    fn language_ids(&self) -> &[&str] {
        &["prolog"]
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
            // target_name is the module path (e.g., "library(lists)" or a file path).
            let module_path = r.module.clone().or_else(|| Some(r.target_name.clone()));
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path,
                alias: None,
                // use_module without an import list brings in all public predicates.
                is_wildcard: true,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "prolog".to_string(),
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

        // Prolog standard builtins are never in the index.
        if predicates::is_prolog_builtin(target) {
            return None;
        }

        // Strip module qualification: `lists:member` → `member`.
        // resolve_common operates on target_name directly; when the target
        // contains a module qualifier we must resolve the bare predicate name.
        let effective_target = target.split(':').last().unwrap_or(target);

        if effective_target != target.as_str() {
            // Qualified form: scope chain then same-file with the bare predicate.
            for scope in &ref_ctx.scope_chain {
                let candidate = format!("{scope}.{effective_target}");
                if let Some(sym) = lookup.by_qualified_name(&candidate) {
                    if predicates::kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 1.0,
                            strategy: "prolog_scope_chain",
                            resolved_yield_type: None,
                        });
                    }
                }
            }
            for sym in lookup.in_file(&file_ctx.file_path) {
                if sym.name == effective_target && predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "prolog_same_file",
                        resolved_yield_type: None,
                    });
                }
            }
            return None;
        }

        engine::resolve_common("prolog", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, predicates::is_prolog_builtin)
    }
}
