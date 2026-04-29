// =============================================================================
// lua/resolve.rs — Lua resolution rules
//
// Scope rules for Lua:
//
//   1. Scope chain walk: innermost function/block → outermost.
//   2. Same-file resolution: all top-level symbols are visible within the file.
//   3. By-name lookup: for required modules, symbols may be defined elsewhere.
//
// Lua import model:
//   `require("module")`   → target_name = "module"
//   `require("dir.sub")`  → target_name = "dir.sub"
//
// The extractor emits EdgeKind::Imports with target_name = the module string.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

/// Lua language resolver.
pub struct LuaResolver;

impl LanguageResolver for LuaResolver {
    fn language_ids(&self) -> &[&str] {
        &["lua"]
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
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(r.target_name.clone()),
                alias: None,
                // Lua require("module") returns a table; subsequent calls like
                // `module.func()` use the module as a prefix. Mark as wildcard
                // so the import walk classifies unresolved bare names from modules.
                is_wildcard: true,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "lua".to_string(),
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

        // Lua builtins are never in the index.
        if predicates::is_lua_builtin(target) {
            return None;
        }

        if let Some(res) = engine::resolve_common(
            "lua", file_ctx, ref_ctx, lookup, predicates::kind_compatible,
        ) {
            return Some(res);
        }

        // Lua bare-name fallback. 8th language using the
        // `<lang>_bare_name` template (PRs 31, 35-40). Lua's
        // require()-loaded modules and `M.X` table dispatch leave
        // many calls without an explicit module qualifier the engine
        // can bind. Index-wide name lookup gated by `.lua`/`.luac`
        // file-extension and `kind_compatible`.
        if matches!(edge_kind, EdgeKind::Calls | EdgeKind::TypeRef | EdgeKind::Instantiates)
            && ref_ctx.extracted_ref.module.is_none()
            && !target.contains('.')
            && !target.contains(':')
        {
            for sym in lookup.by_name(target) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                let path = &sym.file_path;
                let is_lua = path.ends_with(".lua") || path.ends_with(".luac");
                if !is_lua {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.80,
                    strategy: "lua_bare_name",
                    resolved_yield_type: None,
                });
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, predicates::is_lua_builtin)
    }
}
