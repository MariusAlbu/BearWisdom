// =============================================================================
// r_lang/resolve.rs — R resolution rules
//
// Scope rules for R:
//
//   1. Scope chain walk: innermost function → outermost (lexical scoping).
//   2. Same-file resolution: all top-level assignments are visible within
//      the file.
//   3. By-name lookup: for loaded packages, symbols may be defined elsewhere.
//
// R import model:
//   `library(pkg)`       → target_name = "pkg"
//   `require(pkg)`       → target_name = "pkg"
//   `source("file.R")`   → target_name = "file.R"
//
// The extractor emits EdgeKind::Imports with target_name = the package name
// or sourced file path.
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    self, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// R language resolver.
pub struct RResolver;

impl LanguageResolver for RResolver {
    fn language_ids(&self) -> &[&str] {
        &["r"]
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
            language: "r".to_string(),
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
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return None;
        }

        // R builtins are never in the index.
        if builtins::is_r_builtin(&ref_ctx.extracted_ref.target_name) {
            return None;
        }

        engine::resolve_common("r", file_ctx, ref_ctx, lookup, builtins::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, builtins::is_r_builtin)
    }
}
