// =============================================================================
// zig/resolve.rs — Zig resolution rules
//
// Scope rules for Zig:
//
//   1. Scope chain walk: innermost fn/struct → outermost.
//   2. Same-file resolution: all top-level declarations visible within the file.
//   3. Import-based resolution:
//        `const mod = @import("module.zig")` → brings `mod` into scope
//        `const std = @import("std")`        → standard library (external)
//
// The extractor emits EdgeKind::Imports with:
//   target_name = the local binding name (e.g., "std", "mod")
//   module      = the @import argument string (e.g., "std", "module.zig")
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Zig language resolver.
pub struct ZigResolver;

impl LanguageResolver for ZigResolver {
    fn language_ids(&self) -> &[&str] {
        &["zig"]
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
            // target_name = local alias (const name), module = @import argument
            let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            let alias = if r.module.is_some() {
                Some(r.target_name.clone())
            } else {
                None
            };

            imports.push(ImportEntry {
                imported_name: module_path.clone(),
                module_path: Some(module_path),
                alias,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "zig".to_string(),
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

        if builtins::is_zig_builtin(target) {
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
                        strategy: "zig_scope_chain",
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
                    strategy: "zig_same_file",
                });
            }
        }

        // Step 3: Simple name lookup across the project.
        for sym in lookup.by_name(target) {
            if builtins::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "zig_by_name",
                });
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let uri = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            // @import("std") and other non-relative imports are external.
            if !uri.starts_with('.') && !uri.ends_with(".zig") {
                return Some(format!("zig.{uri}"));
            }
            return None;
        }

        if builtins::is_zig_builtin(target) {
            return Some("zig.builtin".to_string());
        }

        // Alias-qualified: `std.mem.Allocator` → check if `std` is an external import.
        let root = target.split('.').next().unwrap_or(target);
        for import in &file_ctx.imports {
            if let Some(alias) = &import.alias {
                if alias == root {
                    let uri = import.module_path.as_deref().unwrap_or("");
                    if !uri.starts_with('.') && !uri.ends_with(".zig") {
                        return Some(format!("zig.{uri}"));
                    }
                }
            }
        }

        None
    }
}
