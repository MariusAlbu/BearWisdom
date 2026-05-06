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

use super::predicates;
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
                // Erlang imports bring module-level scope: `-import(lists, [map/2])` or
                // remote calls `lists:map(...)` mean all exports are potentially available.
                // Mark as wildcard so the import walk can classify bare unresolved names.
                is_wildcard: true,
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

        // Bare-name walker lookup. erlang_otp emits real symbols for
        // BIFs (length, hd, tl, spawn) and stdlib modules (lists, gen_server,
        // gen_statem, supervisor). ext:-only filter so resolve_common's
        // import / scope / same-file paths still win for project symbols.
        if !target.contains(':') {
            for sym in lookup.by_name(target) {
                if !sym.file_path.starts_with("ext:") {
                    continue;
                }
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "erlang_synthetic_global",
                    resolved_yield_type: None,
                });
            }
        }

        // Run common resolution first.
        if let Some(res) = engine::resolve_common("erlang", file_ctx, ref_ctx, lookup, predicates::kind_compatible) {
            return Some(res);
        }

        // Arity-aware same-file fallback.
        //
        // Erlang function symbols are indexed as "name/arity" (e.g. "loop/1"),
        // but call-site refs carry only the bare name ("loop"). resolve_common
        // step 4 does an exact name match which fails for every Erlang function.
        // We retry here by comparing the target against the base name before the
        // `/` in each same-file symbol name.
        if edge_kind == EdgeKind::Calls {
            let mut best: Option<Resolution> = None;
            for sym in lookup.in_file(&file_ctx.file_path) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                // sym.name is "foo/N" for Erlang functions.
                let base = sym.name.split('/').next().unwrap_or(&sym.name);
                if base == target.as_str() {
                    // Prefer the first match; all arities of the same function
                    // in the same file are equally valid at this confidence level.
                    if best.is_none() {
                        best = Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.9,
                            strategy: "erlang_same_file_arity",
                            resolved_yield_type: None,
                        });
                    }
                }
            }
            if best.is_some() {
                return best;
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // erlang_otp walker emits real symbols and resolve() above binds
        // them. Names that exhaust resolve() stay unresolved rather than
        // being blanket-classified as `builtin`.
        None
    }
}
