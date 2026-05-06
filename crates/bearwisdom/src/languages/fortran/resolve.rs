// =============================================================================
// fortran/resolve.rs — Fortran resolution rules
//
// Scope rules for Fortran:
//
//   1. Scope chain walk: innermost subroutine/function → module → program.
//   2. Same-file resolution: all top-level procedures visible within the file.
//   3. Import-based resolution:
//        `use module_name`                   → all public symbols from module
//        `use module_name, only: sym1, sym2` → only named symbols
//
// The extractor emits EdgeKind::Imports with:
//   target_name = module name (or specific symbol for `only:` clauses)
//   module      = module name when target_name is a renamed/only symbol
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Fortran language resolver.
pub struct FortranResolver;

impl LanguageResolver for FortranResolver {
    fn language_ids(&self) -> &[&str] {
        &["fortran"]
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
            // `use module_name` → target_name = module_name, module = None
            // `use module_name, only: sym` → target_name = sym, module = module_name
            let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            let is_wildcard = r.module.is_none(); // plain `use` = wildcard

            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias: None,
                is_wildcard,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "fortran".to_string(),
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

        let target = &ref_ctx.extracted_ref.target_name;
        let target_lower = target.to_lowercase();

        // Fortran is case-insensitive: check same-file with lowercased comparison
        // before delegating to resolve_common (which is case-sensitive).
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name.to_lowercase() == target_lower
                && predicates::kind_compatible(ref_ctx.extracted_ref.kind, &sym.kind)
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "fortran_same_file",
                    resolved_yield_type: None,
                });
            }
        }

        engine::resolve_common("fortran", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // Fortran intrinsics + type specifiers + control-flow keywords
        // are classified by the engine's keywords() set (case-sensitive
        // match — KEYWORDS holds the lowercase forms; refs are normalised
        // upstream).
        None
    }
}
