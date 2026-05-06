// =============================================================================
// haskell/resolve.rs — Haskell resolution rules
//
// Scope rules for Haskell:
//
//   1. Scope chain walk: innermost where/let → top-level.
//   2. Same-file resolution: all top-level bindings in the module are visible.
//   3. Import-based resolution:
//        `import Module`                  → wildcard import
//        `import qualified Module as M`   → aliased qualified import
//        `import Module (sym1, sym2)`     → selective import
//        `import Module hiding (sym)`     → hiding (treated as wildcard here)
//
// Haskell import model:
//   target_name = the module name (or alias when `as` is used)
//   module      = the original module name when an alias is present
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Haskell language resolver.
pub struct HaskellResolver;

impl LanguageResolver for HaskellResolver {
    fn language_ids(&self) -> &[&str] {
        &["haskell"]
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
            // For Haskell, target_name is the module name or alias.
            // module is the original module name when an alias is present.
            let module_name = r.module.as_deref().unwrap_or(&r.target_name);
            let alias = if r.module.is_some() && r.target_name != module_name {
                Some(r.target_name.clone())
            } else {
                None
            };

            imports.push(ImportEntry {
                imported_name: module_name.to_string(),
                module_path: Some(module_name.to_string()),
                alias,
                is_wildcard: true,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "haskell".to_string(),
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

        // Bare-name walker lookup. cabal walks Hackage source jars when the
        // project's *.cabal declares a dep; Prelude / base / containers /
        // text symbols emit under ext:cabal:base/... Skip when chain
        // context is present.
        if ref_ctx.extracted_ref.chain.is_none() && !target.contains('.') {
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
                    strategy: "haskell_synthetic_global",
                    resolved_yield_type: None,
                });
            }
        }

        engine::resolve_common("haskell", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // cabal walker emits real symbols and resolve() above binds them;
        // names that exhaust resolve() stay unresolved rather than blanket-
        // classified as `builtin`.
        None
    }
}
