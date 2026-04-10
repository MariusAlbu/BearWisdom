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

use super::builtins;
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

        // Haskell Prelude is always in scope and not in the project index.
        if builtins::is_haskell_builtin(target) {
            return None;
        }

        engine::resolve_common("haskell", file_ctx, ref_ctx, lookup, builtins::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, builtins::is_haskell_builtin)
    }
}
