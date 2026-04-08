// =============================================================================
// ocaml/resolve.rs — OCaml resolution rules
//
// Scope rules for OCaml:
//
//   1. Scope chain walk: innermost let/module → top-level.
//   2. Same-file resolution: all top-level bindings and modules are visible.
//   3. Import-based resolution:
//        `open Module`       → wildcard open; all public names in scope
//        `include Module`    → structural include (treated as wildcard open)
//        `module M = Module` → alias (M is a local name for Module)
//
// OCaml import model:
//   target_name = the module being opened/included or the local alias
//   module      = the source module when an alias is introduced
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// OCaml language resolver.
pub struct OcamlResolver;

impl LanguageResolver for OcamlResolver {
    fn language_ids(&self) -> &[&str] {
        &["ocaml"]
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
            // target_name is the opened/included module or local alias.
            // module is the original module when an alias is present.
            let source_module = r.module.as_deref().unwrap_or(&r.target_name);
            let alias = if r.module.is_some() && r.target_name != source_module {
                Some(r.target_name.clone())
            } else {
                None
            };

            imports.push(ImportEntry {
                imported_name: source_module.to_string(),
                module_path: Some(source_module.to_string()),
                alias,
                is_wildcard: true,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "ocaml".to_string(),
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

        // OCaml Stdlib is always in scope and not in the project index.
        if builtins::is_ocaml_builtin(target) {
            return None;
        }

        engine::resolve_common("ocaml", file_ctx, ref_ctx, lookup, builtins::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, builtins::is_ocaml_builtin)
    }
}
