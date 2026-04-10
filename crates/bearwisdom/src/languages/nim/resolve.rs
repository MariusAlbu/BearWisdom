// =============================================================================
// nim/resolve.rs — Nim resolution rules
//
// Scope rules for Nim:
//
//   1. Scope chain walk: innermost proc/type → outermost.
//   2. Same-file resolution: all top-level symbols visible within the file.
//   3. Import-based resolution:
//        `import module`            → all exported symbols from module
//        `from module import sym`   → only named symbols
//        `include file`             → textual inclusion, all symbols visible
//
// The extractor emits EdgeKind::Imports with:
//   target_name = module name or symbol name
//   module      = module path for `from ... import` forms
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Nim language resolver.
pub struct NimResolver;

impl LanguageResolver for NimResolver {
    fn language_ids(&self) -> &[&str] {
        &["nim"]
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
            // `from module import sym` → module is in r.module, sym in r.target_name
            // `import module`          → module name is r.target_name
            let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());

            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias: None,
                is_wildcard: r.module.is_none(), // plain `import` = wildcard
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "nim".to_string(),
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

        if builtins::is_nim_builtin(target) {
            return None;
        }

        engine::resolve_common("nim", file_ctx, ref_ctx, lookup, builtins::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, builtins::is_nim_builtin)
    }
}
