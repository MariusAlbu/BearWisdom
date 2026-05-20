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

        // Bare-name walker lookup. nvim_runtime walker covers vim.* API
        // when LanguagePresent("lua") + a Neovim install is probed;
        // luarocks walker covers declared *.rockspec deps. Interpreter
        // built-ins (print, pairs, ipairs, table.insert, ...) are
        // handled by the engine's keywords() set. ext:-only filter so
        // resolve_common's standard paths still win for project symbols.
        if !target.contains('.') && !target.contains(':') {
            for sym in lookup.by_name(target) {
                if !sym.file_path.starts_with("ext:") {
                    continue;
                }
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "lua_synthetic_global",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
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
                    flow_emit: None,
                });
            }
        }

        None
    }


    fn detect_flow_emission(
        &self,
        _file_ctx: &FileContext,
        ref_ctx: &RefContext,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let r = &ref_ctx.extracted_ref;
        if r.kind != EdgeKind::Calls {
            return Vec::new();
        }
        let module = r.module.as_deref().unwrap_or("");
        let target = r.target_name.as_str();
        if let Some(em) = detect_lua_lapis_route(module, target, &r.call_args) {
            return vec![em];
        }
        if let Some(em) = detect_lua_resty_http(module, target, &r.call_args) {
            return vec![em];
        }
        if let Some(em) = detect_lua_db_emission(module, target) {
            return vec![em];
        }
        Vec::new()
    }
}

pub(crate) fn detect_lua_lapis_route(
    module: &str,
    target_name: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    // Lapis: `app:get("/x", ...)`, `app:post(...)`. The module is the
    // application name (varies). Use target_name to detect.
    let _ = module;
    let method = match target_name {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "delete" => HttpMethod::Delete,
        "match" => HttpMethod::Any,
        _ => return None,
    };
    let url = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s) if s.starts_with('/') => Some(s.as_str()),
        _ => None,
    })?;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: crate::connectors::url_pattern::normalize(url),
        role: ChannelRole::Consumer,
        method: Some(method),
    streaming: None,
    })
}

pub(crate) fn detect_lua_resty_http(
    module: &str,
    target: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    if !module.contains("resty.http") && module != "socket.http" {
        return None;
    }
    if !matches!(target, "request_uri" | "request" | "get" | "post") {
        return None;
    }
    let url = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s)
            if s.starts_with('/') || s.starts_with("http://") || s.starts_with("https://") =>
        {
            Some(s.as_str())
        }
        _ => None,
    })?;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: crate::connectors::url_pattern::normalize(url),
        role: ChannelRole::Producer,
        method: Some(HttpMethod::Any),
    streaming: None,
    })
}

pub(crate) fn detect_lua_db_emission(
    module: &str,
    target: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    if !module.contains("pgmoon") && !module.contains("luasql") && !module.contains("resty.mysql") {
        return None;
    }
    let op = match target {
        "query" => DbQueryOp::Select,
        "execute" => DbQueryOp::Other,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: "lua.*".to_string(),
        operation: op,
    })
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
