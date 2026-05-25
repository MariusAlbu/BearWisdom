// Gleam language hooks. Absorbed from the deleted `gleam/resolve.rs`.

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct GleamHooks;

fn is_gleam_operator(name: &str) -> bool {
    matches!(
        name,
        "+" | "-" | "*" | "/" | "%" | "==" | "!=" | "<" | "<=" | ">" | ">="
            | "&&" | "||" | "!" | "|>" | "<>" | "+." | "-." | "*." | "/."
            | "==." | "!=." | "<." | "<=." | ">." | ">=."
    )
}

pub(crate) fn detect_gleam_http_producer(
    module: &str,
    target: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    if !module.contains("httpc") && !module.contains("gleam/http") {
        return None;
    }
    let method = match target {
        "send" => HttpMethod::Any,
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
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
    });
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: url
            .map(crate::connectors::url_pattern::normalize)
            .unwrap_or_else(|| "*".to_string()),
        role: ChannelRole::Producer,
        method: Some(method),
        streaming: None,
    })
}

pub(crate) fn detect_gleam_pgo_emission(
    module: &str,
    target: &str,
    _call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    if !module.contains("pgo") && !module.contains("pog") && !module.contains("sqlight") {
        return None;
    }
    let op = match target {
        "execute" | "query" => DbQueryOp::Other,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: "gleam.*".to_string(),
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
    if let Some(em) = detect_gleam_http_producer(module, target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_gleam_pgo_emission(module, target, &r.call_args) {
        return vec![em];
    }
    Vec::new()
}

impl LanguageEngineHooks for GleamHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        engine::infer_external_common(file_ctx, ref_ctx, project_ctx, |_| false)
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
            let module_path = r.module.clone().unwrap_or_else(|| r.target_name.clone());
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module_path),
                alias: None,
                is_wildcard: true,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "gleam".to_string(),
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
        if is_gleam_operator(target) {
            return None;
        }
        for import in &file_ctx.imports {
            let Some(full_path) = &import.module_path else {
                continue;
            };
            let module_alias = import.alias.as_deref().unwrap_or_else(|| {
                full_path.rsplit('/').next().unwrap_or(full_path.as_str())
            });
            let candidate = format!("{module_alias}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "gleam_import_qualified",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
            for sym in lookup.in_file(full_path) {
                if sym.name == *target {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "gleam_import_file",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
        (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: |_, _| true,
        })
        .resolve_all()
    }
}

pub static GLEAM_HOOKS: GleamHooks = GleamHooks;
