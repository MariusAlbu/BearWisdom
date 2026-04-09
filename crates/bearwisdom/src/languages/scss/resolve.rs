// =============================================================================
// languages/scss/resolve.rs — SCSS resolution rules
//
// SCSS reference forms emitted by the extractor:
//
//   @include mixin-name(args)   → Calls,  target_name = "mixin-name",  module = None
//   @extend %placeholder        → Inherits, target_name = "%placeholder", module = None
//   @import 'path'              → Imports, target_name = last segment,  module = "path"
//   @use 'path' as alias        → Imports, target_name = last segment,  module = "path"
//   @forward 'path'             → Imports, target_name = last segment,  module = "path"
//   call_expression (fn call)   → Calls,  target_name = "function-name", module = None
//
// Resolution strategy:
//   1. Imports (@use / @import / @forward): record the module path in file
//      context. These are file-level declarations, not symbol references.
//   2. Mixin/function calls (@include, direct calls): look up the target name
//      via `lookup.by_name()`. SCSS symbols have bare names as qualified_name.
//   3. Same-file: mixin defined in the same file is always visible.
//   4. CSS built-in functions (color(), rgba(), etc.) are external.
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct ScssResolver;

impl LanguageResolver for ScssResolver {
    fn language_ids(&self) -> &[&str] {
        &["scss"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        // Collect @use / @import / @forward paths from Imports refs.
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias: None,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "scss".to_string(),
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

        // Skip import declarations — they declare scope, not symbol references.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Skip CSS built-in functions.
        if builtins::is_scss_builtin(target) {
            return None;
        }

        engine::resolve_common("scss", file_ctx, ref_ctx, lookup, builtins::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, builtins::is_scss_builtin)
    }
}
