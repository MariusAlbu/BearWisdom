// =============================================================================
// nim/resolve.rs — Nim resolution rules
//
// Scope rules for Nim:
//
//   1. Scope chain walk: innermost proc/type → outermost.
//   2. Same-file resolution: all top-level symbols visible within the file.
//   3. Import-based resolution:
//        `import module`            → all exported symbols from module
//        `from module import sym`   → only named symbols
//        `include file`             → textual inclusion, all symbols visible
//
// The extractor emits EdgeKind::Imports with:
//   target_name = module name or symbol name
//   module      = module path for `from ... import` forms
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Nim language resolver.
pub struct NimResolver;

impl LanguageResolver for NimResolver {
    fn language_ids(&self) -> &[&str] {
        &["nim"]
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
            // `from module import sym` → module is in r.module, sym in r.target_name
            // `import module`          → module name is r.target_name
            let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());

            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias: None,
                is_wildcard: r.module.is_none(), // plain `import` = wildcard
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "nim".to_string(),
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

        if builtins::is_nim_builtin(target) {
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
                        strategy: "nim_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-file resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == *target && builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "nim_same_file",
                });
            }
        }

        // Step 3: Simple name lookup across the project.
        for sym in lookup.by_name(target) {
            if builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "nim_by_name",
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

        if builtins::is_nim_builtin(target) {
            return Some("nim.stdlib".to_string());
        }

        None
    }
}
