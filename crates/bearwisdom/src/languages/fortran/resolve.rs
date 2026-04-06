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

use super::builtins;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
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
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Fortran is case-insensitive; check both as-is and lowercased.
        let target_lower = target.to_lowercase();
        if builtins::is_fortran_builtin(target) || builtins::is_fortran_builtin(&target_lower) {
            return None;
        }

        // Step 1: Scope chain walk.
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if builtins::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "fortran_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name.to_lowercase() == target_lower
                && builtins::kind_compatible(edge_kind, &sym.kind)
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "fortran_same_file",
                });
            }
        }

        // Step 3: Simple name lookup across the project (case-insensitive).
        for sym in lookup.by_name(target) {
            if builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "fortran_by_name",
                });
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return None;
        }

        let target_lower = target.to_lowercase();
        if builtins::is_fortran_builtin(target) || builtins::is_fortran_builtin(&target_lower) {
            return Some("fortran.intrinsic".to_string());
        }

        None
    }
}
