// =============================================================================
// pascal/resolve.rs — Pascal/Delphi resolution rules
//
// Scope rules for Pascal/Delphi:
//
//   1. Scope chain walk: innermost procedure/function → class → unit.
//   2. Same-file resolution: all declarations in the same unit are visible.
//   3. Import-based resolution:
//        `uses Unit1, Unit2;` → all public symbols from each unit enter scope
//
// The extractor emits EdgeKind::Imports with:
//   target_name = unit name (e.g., "SysUtils", "Classes")
//   module      = None (Pascal `uses` clauses always name the unit directly)
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Pascal/Delphi language resolver.
pub struct PascalResolver;

impl LanguageResolver for PascalResolver {
    fn language_ids(&self) -> &[&str] {
        &["pascal", "delphi"]
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
            // `uses UnitName` → each unit is a wildcard import (all public names visible).
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(r.target_name.clone()),
                alias: None,
                is_wildcard: true,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "pascal".to_string(),
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

        if predicates::is_pascal_builtin(&ref_ctx.extracted_ref.target_name) {
            return None;
        }

        // Pascal is case-insensitive: check same-file with lowercased comparison
        // before delegating to resolve_common (which is case-sensitive).
        let target_lower = ref_ctx.extracted_ref.target_name.to_lowercase();
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name.to_lowercase() == target_lower
                && predicates::kind_compatible(ref_ctx.extracted_ref.kind, &sym.kind)
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "pascal_same_file",
                });
            }
        }

        engine::resolve_common("pascal", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, predicates::is_pascal_builtin)
    }
}
