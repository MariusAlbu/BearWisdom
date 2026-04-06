// =============================================================================
// clojure/resolve.rs — Clojure resolution rules
//
// Scope rules for Clojure:
//
//   1. Scope chain walk: innermost let/letfn → defn → ns.
//   2. Same-file resolution: all top-level vars/defs in the namespace are visible.
//   3. Import-based resolution:
//        `(ns my.ns (:require [lib :as l]))` → aliased require
//        `(require '[lib :as l])`            → aliased require
//        `(use 'lib)`                        → wildcard use
//        `(import '(java.util Date))`        → Java class import
//
// Clojure import model:
//   target_name = the local alias or namespace name
//   module      = the canonical namespace when an alias is present
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Clojure language resolver.
pub struct ClojureResolver;

impl LanguageResolver for ClojureResolver {
    fn language_ids(&self) -> &[&str] {
        &["clojure"]
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
            // target_name is the local alias or the full namespace.
            // module is the canonical namespace when an alias is present.
            let ns = r.module.as_deref().unwrap_or(&r.target_name);
            let alias = if r.module.is_some() && r.target_name != ns {
                Some(r.target_name.clone())
            } else {
                None
            };

            let is_wildcard = alias.is_none();
            imports.push(ImportEntry {
                imported_name: ns.to_string(),
                module_path: Some(ns.to_string()),
                alias,
                is_wildcard,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "clojure".to_string(),
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

        // clojure.core and special forms are not in the project index.
        if builtins::is_clojure_builtin(target) {
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
                        strategy: "clojure_scope_chain",
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
                    strategy: "clojure_same_file",
                });
            }
        }

        // Step 3: Simple name lookup across the project.
        for sym in lookup.by_name(target) {
            if builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "clojure_by_name",
                });
            }
        }

        None
    }
}
