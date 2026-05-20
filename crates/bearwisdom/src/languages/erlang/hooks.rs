// Erlang language hooks. Absorbed from the deleted `erlang/resolve.rs`.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct ErlangHooks;

pub(crate) fn detect_erlang_http_emission(
    module: &str,
    target: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    let method = match (module, target) {
        ("httpc", "request") => HttpMethod::Any,
        ("hackney", "get") => HttpMethod::Get,
        ("hackney", "post") => HttpMethod::Post,
        ("hackney", "put") => HttpMethod::Put,
        ("hackney", "delete") => HttpMethod::Delete,
        ("hackney", "patch") => HttpMethod::Patch,
        _ => return None,
    };
    let url = call_args.iter().find_map(|a| match a {
        CallArg::StringLit(s)
            if s.starts_with('/')
                || s.starts_with("http://")
                || s.starts_with("https://") =>
        {
            Some(s.as_str())
        }
        _ => None,
    })?;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: crate::connectors::url_pattern::normalize(url),
        role: ChannelRole::Producer,
        method: Some(method),
        streaming: None,
    })
}

pub(crate) fn detect_erlang_db_emission(
    module: &str,
    target: &str,
    _call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let op = match (module, target) {
        ("epgsql", "equery") | ("epgsql", "squery") | ("epgsql", "execute") => DbQueryOp::Other,
        ("mnesia", "read")
        | ("mnesia", "match_object")
        | ("mnesia", "select")
        | ("mnesia", "dirty_read") => DbQueryOp::Select,
        ("mnesia", "write") | ("mnesia", "dirty_write") => DbQueryOp::Insert,
        ("mnesia", "delete") | ("mnesia", "dirty_delete") => DbQueryOp::Delete,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: "erl.*".to_string(),
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
    let raw_target = r.target_name.as_str();
    let target = raw_target.split('/').next().unwrap_or(raw_target);
    if let Some(em) = detect_erlang_http_emission(module, target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_erlang_db_emission(module, target, &r.call_args) {
        return vec![em];
    }
    Vec::new()
}

fn resolve_via_import(
    file_ctx: &FileContext,
    target: &str,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    for entry in &file_ctx.imports {
        if entry.imported_name != target {
            continue;
        }
        let source_mod = entry.module_path.as_deref()?;
        for sym in lookup.by_name(target) {
            let path = sym.file_path.as_ref();
            let matches = path.contains(source_mod)
                || path
                    .rsplit('/')
                    .next()
                    .and_then(|f| f.strip_suffix(".erl"))
                    .map(|stem| stem == source_mod)
                    .unwrap_or(false);
            if matches && predicates::kind_compatible(EdgeKind::Calls, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.92,
                    strategy: "erlang_import_arity",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }
    None
}

impl LanguageEngineHooks for ErlangHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        _project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        if ref_ctx.extracted_ref.kind != EdgeKind::Calls {
            return None;
        }
        let target = &ref_ctx.extracted_ref.target_name;
        let bare = target.split('/').next().unwrap_or(target.as_str());
        if bare.is_empty() {
            return None;
        }
        let plugin_keywords = crate::indexer::keywords::keywords_for_language("erlang");
        if plugin_keywords.contains(&bare) {
            return Some("primitive".to_string());
        }
        if super::keywords::KEYWORDS.contains(&bare) {
            return Some("builtin".to_string());
        }
        None
    }

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
            let is_function_import = r.target_name.contains('/');
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: r.module.clone().or_else(|| Some(r.target_name.clone())),
                alias: None,
                is_wildcard: !is_function_import,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "erlang".to_string(),
            imports,
            file_namespace: None,
        })
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;
        if edge_kind == EdgeKind::Imports {
            return None;
        }
        if edge_kind == EdgeKind::Calls && !target.contains(':') {
            for sym in lookup.by_name(target) {
                if !sym.file_path.starts_with("ext:erlang:") {
                    continue;
                }
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "erlang_otp_arity",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        if edge_kind == EdgeKind::Calls && !target.contains(':') {
            if let Some(res) = resolve_via_import(file_ctx, target, lookup) {
                return Some(res);
            }
        }
        if let Some(res) = engine::resolve_common(
            "erlang",
            file_ctx,
            ref_ctx,
            lookup,
            predicates::kind_compatible,
        ) {
            return Some(res);
        }
        if edge_kind == EdgeKind::Calls {
            for sym in lookup.in_file(&file_ctx.file_path) {
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                if sym.name == target.as_str() {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.9,
                        strategy: "erlang_same_file_arity",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
            let target_base = target.split('/').next().unwrap_or(target.as_str());
            if !target_base.is_empty() && !target.contains('/') {
                for sym in lookup.in_file(&file_ctx.file_path) {
                    if !predicates::kind_compatible(edge_kind, &sym.kind) {
                        continue;
                    }
                    let sym_base = sym.name.split('/').next().unwrap_or(&sym.name);
                    if sym_base == target_base {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.8,
                            strategy: "erlang_same_file_base",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
        }
        if edge_kind == EdgeKind::Calls && !target.contains(':') {
            for sym in lookup.by_name(target) {
                if sym.file_path.starts_with("ext:") {
                    continue;
                }
                if sym.file_path.as_ref() == file_ctx.file_path.as_str() {
                    continue;
                }
                if !predicates::kind_compatible(edge_kind, &sym.kind) {
                    continue;
                }
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.85,
                    strategy: "erlang_cross_file_arity",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        None
    }
}

pub static ERLANG_HOOKS: ErlangHooks = ErlangHooks;
