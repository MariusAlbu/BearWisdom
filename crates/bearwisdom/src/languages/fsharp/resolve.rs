// =============================================================================
// fsharp/resolve.rs — F# resolution rules
//
// Scope rules for F#:
//
//   1. Scope chain walk: innermost let-binding / function → module → namespace.
//   2. Same-file resolution: all top-level bindings in the file are visible.
//   3. Import-based resolution: `open Namespace.Module` and
//      `open type TypeName` bring symbols into scope.
//
// F# import model:
//   `open Namespace.Module`   → wildcard open; all public members in scope
//   `open type TypeName`      → static members of a type in scope
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// F# language resolver.
pub struct FSharpResolver;

impl LanguageResolver for FSharpResolver {
    fn language_ids(&self) -> &[&str] {
        &["fsharp"]
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
            // target_name is the opened namespace/module path.
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(r.target_name.clone()),
                alias: None,
                is_wildcard: true,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "fsharp".to_string(),
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

        // F# stdlib and language keywords are not in the project index.
        if builtins::is_fsharp_builtin(target) {
            return None;
        }

        engine::resolve_common("fsharp", file_ctx, ref_ctx, lookup, builtins::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, builtins::is_fsharp_builtin)
    }
}
