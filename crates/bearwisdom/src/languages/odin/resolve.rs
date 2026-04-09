// =============================================================================
// languages/odin/resolve.rs — Odin resolution rules
//
// Odin package system:
//
//   import "core:fmt"             → Imports, target_name = "core:fmt"  (path), module = None
//   import str "core:strings"     → Imports, target_name = "core:strings", module = None
//   fmt.println("hello")         → Calls, target_name = "println" (pkg qualifier stripped)
//   local_proc()                  → Calls, target_name = "local_proc"
//   using pkg                     → TypeRef, target_name = "pkg"
//
// The extractor strips package qualifiers from call sites: `fmt.println` → "println".
// Import refs carry the package path as `target_name` with no `module` field.
// Import symbols are emitted as `Namespace` symbols with the package name as their name.
//
// Resolution strategy:
//   1. Same-file: procedures/types defined in the same file are always visible.
//   2. Global name lookup: Odin procedures use bare names as qualified_name.
//   3. Scope chain walk for methods.
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct OdinResolver;

impl LanguageResolver for OdinResolver {
    fn language_ids(&self) -> &[&str] {
        &["odin"]
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
            // Odin: target_name holds the import path (e.g., "core:fmt").
            // Derive the package name as the last segment after `:` or `/`.
            let import_path = r.target_name.clone();
            let pkg_name = import_path
                .rsplit(':')
                .next()
                .and_then(|s| s.rsplit('/').next())
                .unwrap_or(import_path.as_str())
                .to_string();

            imports.push(ImportEntry {
                imported_name: pkg_name,
                module_path: Some(import_path),
                alias: None,
                is_wildcard: false,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "odin".to_string(),
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

        // Skip import declarations.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Skip Odin built-in types.
        if builtins::is_odin_builtin(target) {
            return None;
        }

        engine::resolve_common("odin", file_ctx, ref_ctx, lookup, builtins::kind_compatible)
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Language-specific: Odin core:/vendor:/base: package paths are external.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            if target.starts_with("core:") || target.starts_with("vendor:") || target.starts_with("base:") {
                return Some(target.clone());
            }
        }

        engine::infer_external_common(file_ctx, ref_ctx, builtins::is_odin_builtin)
    }
}
