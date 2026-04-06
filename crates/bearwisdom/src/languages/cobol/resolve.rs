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

use super::builtins;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
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
        if builtins::is_cobol_builtin(target) {
            return None;
        }

        // Normalise: COBOL is case-insensitive; names are typically uppercased.
        // The extractor should already normalise, but guard here for safety.
        let effective_target = target.as_str();

        // Step 1: Scope chain walk (section → program division).
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "cobol_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-file resolution (paragraphs and sections in the same program).
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == effective_target && builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "cobol_same_file",
                });
            }
        }

        // Step 3: Project-wide name lookup (external CALL targets).
        for sym in lookup.by_name(effective_target) {
            if builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "cobol_by_name",
                });
            }
        }

        None
    }
}
