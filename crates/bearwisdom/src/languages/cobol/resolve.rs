// =============================================================================
// cobol/resolve.rs — COBOL resolution rules
//
// COBOL module system:
//   - `COPY copybook.` — textual inclusion of a copybook (like a C #include).
//   - `COPY copybook REPLACING ==old== BY ==new==.` — inclusion with substitution.
//   - `CALL 'program-name'` — dynamic call to another COBOL program (by literal
//     name) or a variable holding a program name.
//   - Paragraphs and sections within the same program are directly callable via
//     PERFORM.
//
// Resolution strategy:
//   1. Scope chain walk (section → program).
//   2. Same-file resolution (paragraphs and sections in the same program unit).
//   3. Project-wide name lookup (external program names from CALL statements).
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// COBOL language resolver.
pub struct CobolResolver;

impl LanguageResolver for CobolResolver {
    fn language_ids(&self) -> &[&str] {
        &["cobol"]
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
            // target_name is the copybook name or CALL target.
            let module_path = r.module.clone().or_else(|| Some(r.target_name.clone()));
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path,
                alias: None,
                // COPY is a textual include — all names become visible.
                is_wildcard: true,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "cobol".to_string(),
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

        // COBOL standard verbs / intrinsics are never in the index.
        if predicates::is_cobol_builtin(target) {
            return None;
        }

        engine::resolve_common("cobol", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, predicates::is_cobol_builtin)
    }
}
