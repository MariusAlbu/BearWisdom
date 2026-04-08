// =============================================================================
// vba/resolve.rs — VBA resolution rules
//
// VBA module system:
//   - No code-level import statements. Dependencies are set via project
//     References (Tools → References in the IDE), which are project-scoped.
//   - `Dim x As New ClassName` instantiates a class from the same project or
//     a referenced library.
//   - Modules, class modules, and forms are all top-level name spaces.
//   - `Public` symbols are visible across all modules in the project.
//
// Resolution strategy:
//   1. Scope chain walk (within the current procedure → module).
//   2. Same-file resolution (module-level Public/Private declarations).
//   3. Project-wide name lookup (Public symbols from other modules).
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// VBA language resolver.
pub struct VbaResolver;

impl LanguageResolver for VbaResolver {
    fn language_ids(&self) -> &[&str] {
        &["vba"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        // VBA has no code-level import syntax. The extractor may emit
        // EdgeKind::Imports for `Implements InterfaceName` or similar;
        // collect them for completeness.
        let mut imports = Vec::new();

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: r.module.clone(),
                alias: None,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "vba".to_string(),
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

        // VBA builtins are never in the index.
        if builtins::is_vba_builtin(target) {
            return None;
        }

        engine::resolve_common("vba", file_ctx, ref_ctx, lookup, builtins::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, builtins::is_vba_builtin)
    }
}
