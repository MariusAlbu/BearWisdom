// Lua language hooks. Absorbed from the deleted `lua/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup, RESOLVED_CONFIDENCE,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct LuaHooks;

/// Parse a value-alias signature emitted for `local NAME = TABLE.MEMBER`.
/// Returns `(local_name, dotted_qname)` — e.g. `"floor = math.floor"` yields
/// `("floor", "math.floor")`. Declines a signature whose RHS is not a dotted
/// member reference (a `NAME = {}` table or any non-alias form).
pub(crate) fn parse_value_alias(signature: &str) -> Option<(&str, &str)> {
    let (local, rhs) = signature.split_once(" = ")?;
    let local = local.trim();
    let rhs = rhs.trim();
    if local.is_empty() || !rhs.contains('.') {
        return None;
    }
    // A dotted member reference only — reject table literals / call results
    // that share the `NAME = ...` shape.
    if rhs.contains(['{', '}', '(', ')', ' ']) {
        return None;
    }
    Some((local, rhs))
}

/// Bind a bare call/type ref to a value-aliased qualified symbol.
///
/// `local floor = math.floor` records the dotted RHS in the alias's signature;
/// `build_file_context` surfaces it as a non-wildcard `ImportEntry` whose
/// `module_path` is the dotted qname. A bare `floor(x)` matches the entry by
/// name and binds to the symbol named by that qname — the qname lookup is also
/// the safety gate: a require-module specifier (the only other non-empty
/// module_path source) names no symbol, so it can never bind here.
fn resolve_via_value_alias(
    target_name: &str,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    if target_name.is_empty() || target_name.contains('.') {
        return None;
    }
    for import in &file_ctx.imports {
        if import.is_wildcard || import.imported_name != target_name {
            continue;
        }
        let Some(qname) = import.module_path.as_deref() else {
            continue;
        };
        if let Some(sym) = lookup.by_qualified_name(qname) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: RESOLVED_CONFIDENCE,
                strategy: "lua_value_alias",
                resolved_yield_type: None,
                flow_emit: None,
            });
        }
    }
    None
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

pub(crate) fn detect_flow_inner(
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

impl LanguageEngineHooks for LuaHooks {
    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        _lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        detect_flow_inner(file_ctx, ref_ctx)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let mut imports = Vec::new();
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(r.target_name.clone()),
                alias: None,
                is_wildcard: true,
            });
        }
        // Value aliases (`local floor = math.floor`) — a non-wildcard entry
        // whose module_path is the dotted RHS qname. `resolve_bare_pre` binds a
        // bare call of `imported_name` to the symbol named by that qname.
        for sym in &file.symbols {
            let Some(sig) = sym.signature.as_deref() else {
                continue;
            };
            if let Some((local, qname)) = parse_value_alias(sig) {
                imports.push(ImportEntry {
                    imported_name: local.to_string(),
                    module_path: Some(qname.to_string()),
                    alias: None,
                    is_wildcard: false,
                });
            }
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "lua".to_string(),
            imports,
            file_namespace: None,
        })
    }

    fn resolve_bare_pre(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        // A value-aliased local (`local floor = math.floor; floor(x)`) binds to
        // the qualified stdlib/member symbol ahead of the generic bare-name
        // ladder, so the alias outranks a same-file bind to the local itself.
        // Receiver-typed colon calls carry a multi-segment chain and never reach
        // this bare path, so a typed receiver always outranks the alias.
        if !matches!(
            ref_ctx.extracted_ref.kind,
            EdgeKind::Calls | EdgeKind::TypeRef
        ) {
            return None;
        }
        resolve_via_value_alias(&ref_ctx.extracted_ref.target_name, file_ctx, lookup)
    }
}

pub static LUA_HOOKS: LuaHooks = LuaHooks;
