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

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
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

        // Every Nim module implicitly imports `system`. Adding it as a
        // wildcard entry here lets the common resolver find builtins like
        // `newException`, `echo`, `cast`, and `GC_*` without requiring an
        // explicit `import system` in the source file.
        imports.push(ImportEntry {
            imported_name: "system".to_string(),
            module_path: Some("system".to_string()),
            alias: None,
            is_wildcard: true,
        });

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

        engine::resolve_common("nim", file_ctx, ref_ctx, lookup, predicates::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        // `import std/X` is the Nim stdlib namespace. The compiler's lib/
        // tree publishes those modules but each one is a *file*, not a
        // symbol — so the heuristic can't bind the import edge. Treat
        // `std/<anything>` as external so the import counts as handled.
        let target = &ref_ctx.extracted_ref.target_name;
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            if let Some(rest) = target.strip_prefix("std/") {
                return Some(format!("ext:nim-stdlib:{rest}"));
            }
            // Bracketed form `std/[strutils, os]` and `from std/X import Y`
            // also reach this resolver — recognise both.
            if target.starts_with("std/") {
                return Some("ext:nim-stdlib".to_string());
            }
            // `pkg/<X>` is the Nimble-package namespace shorthand.
            if let Some(rest) = target.strip_prefix("pkg/") {
                return Some(format!("ext:nim-pkg:{rest}"));
            }
        }
        None
    }
}
