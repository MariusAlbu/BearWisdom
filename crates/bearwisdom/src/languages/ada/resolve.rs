// =============================================================================
// ada/resolve.rs — Ada resolution rules
//
// Scope rules for Ada:
//
//   1. Scope chain walk: innermost subprogram/block → package → library.
//   2. Same-file resolution: all declarations in the same compilation unit.
//   3. Import-based resolution:
//        `with Package_Name;` → makes Package_Name visible (dot-qualified)
//        `use Package_Name;`  → brings all exported names into direct scope
//
// The extractor emits EdgeKind::Imports with:
//   target_name = package name (both `with` and `use` clauses)
//   module      = None (Ada imports are always the package name itself)
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Ada language resolver.
pub struct AdaResolver;

impl LanguageResolver for AdaResolver {
    fn language_ids(&self) -> &[&str] {
        &["ada"]
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
            // Both `with` and `use` clauses produce Imports edges.
            // `use` clauses bring names into direct scope (wildcard semantics).
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(r.target_name.clone()),
                alias: None,
                is_wildcard: true, // Ada `use` makes all names visible
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "ada".to_string(),
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

        if predicates::is_ada_builtin(target) {
            return None;
        }

        // Ada identifiers are case-insensitive; check same-file with case folding
        // before delegating to the common resolver (which uses exact matching).
        let simple = target.split('.').last().unwrap_or(target);
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name.to_lowercase() == simple.to_lowercase()
                && predicates::kind_compatible(edge_kind, &sym.kind)
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "ada_same_file_ci",
                });
            }
        }

        engine::resolve_common("ada", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Ada standard library imports are classified by their top-level package name.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let root = target.split('.').next().unwrap_or(target);
            if matches!(root, "Ada" | "System" | "Interfaces" | "GNAT" | "Standard") {
                return Some(root.to_string());
            }
            // Non-stdlib imports: fall through to common handler.
        }

        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, predicates::is_ada_builtin)
    }
}
