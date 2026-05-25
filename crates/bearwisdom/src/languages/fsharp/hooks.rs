// F# language hooks. Absorbed from the deleted `fsharp/resolve.rs`.

use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct FsharpHooks;

pub(crate) fn is_manifest_external_namespace(ctx: &ProjectContext, ns: &str) -> bool {
    let root = ns.split('.').next().unwrap_or(ns);
    if matches!(root, "System" | "Microsoft") {
        return true;
    }
    if let Some(m) = ctx.manifest(ManifestKind::NuGet) {
        if !m.dependencies.is_empty() {
            if m.dependencies.contains(ns) {
                return true;
            }
            for dep in &m.dependencies {
                if ns.starts_with(dep.as_str())
                    && ns.len() > dep.len()
                    && ns.as_bytes()[dep.len()] == b'.'
                {
                    return true;
                }
                if let Some(dep_root) = dep.split('.').next() {
                    if root == dep_root {
                        return true;
                    }
                }
            }
            return false;
        }
    }
    predicates::is_external_namespace_fallback(ns)
}

pub(crate) fn detect_fsharp_di_chain_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    let leaf = chain.segments.last()?;
    if !matches!(
        leaf.name.as_str(),
        "AddScoped" | "AddTransient" | "AddSingleton"
    ) {
        return None;
    }
    Some(FlowEmission::DiBinding {
        service_symbol_id: 0,
        container: Some("dotnet".to_string()),
    })
}

pub(crate) fn detect_fsharp_di_bare_emission(
    target: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::FlowEmission;
    if !matches!(target, "AddScoped" | "AddTransient" | "AddSingleton") {
        return None;
    }
    Some(FlowEmission::DiBinding {
        service_symbol_id: 0,
        container: Some("dotnet".to_string()),
    })
}

pub(crate) fn detect_fsharp_route(
    target_name: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    let method = match target_name {
        "route" | "routef" | "routeStartsWith" => HttpMethod::Any,
        "GET" => HttpMethod::Get,
        "POST" => HttpMethod::Post,
        "PUT" => HttpMethod::Put,
        "DELETE" => HttpMethod::Delete,
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

pub(crate) fn detect_fsharp_http_producer(
    module: &str,
    target: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    let m_last = module.rsplit('.').next().unwrap_or(module);
    if m_last != "Http" {
        return None;
    }
    if !matches!(
        target,
        "AsyncRequestString" | "RequestString" | "AsyncRequest" | "Request"
    ) {
        return None;
    }
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
        method: Some(HttpMethod::Any),
        streaming: None,
    })
}

pub(crate) fn detect_fsharp_db_query(
    module: &str,
    target: &str,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let op = match (module.rsplit('.').next().unwrap_or(module), target) {
        ("Sql", "execute") | ("Sql", "executeAsync") | ("Sql", "executeReader") => {
            DbQueryOp::Other
        }
        ("Sql", "executeRowAsync") | ("Sql", "executeRow") => DbQueryOp::Select,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: "fs.*".to_string(),
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
    if let Some(em) = detect_fsharp_route(target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_fsharp_http_producer(module, target, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_fsharp_db_query(module, target) {
        return vec![em];
    }
    if let Some(chain) = r.chain.as_ref() {
        if let Some(em) = detect_fsharp_di_chain_emission(chain) {
            return vec![em];
        }
    } else if let Some(em) = detect_fsharp_di_bare_emission(target) {
        return vec![em];
    }
    Vec::new()
}

impl LanguageEngineHooks for FsharpHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let external = match project_ctx {
                Some(ctx) => is_manifest_external_namespace(ctx, target),
                None => predicates::is_external_namespace_fallback(target),
            };
            if external {
                let root = target.split('.').next().unwrap_or(target);
                return Some(root.to_string());
            }
            return None;
        }
        if let Some(module) = &ref_ctx.extracted_ref.module {
            let external = match project_ctx {
                Some(ctx) => is_manifest_external_namespace(ctx, module),
                None => predicates::is_external_namespace_fallback(module),
            };
            if external {
                let root = module.split('.').next().unwrap_or(module);
                return Some(root.to_string());
            }
        }
        for import in &file_ctx.imports {
            let Some(module_path) = &import.module_path else {
                continue;
            };
            let external = match project_ctx {
                Some(ctx) => is_manifest_external_namespace(ctx, module_path),
                None => predicates::is_external_namespace_fallback(module_path),
            };
            if external {
                let root = module_path.split('.').next().unwrap_or(module_path);
                return Some(root.to_string());
            }
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
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(r.target_name.clone()),
                alias: None,
                is_wildcard: true,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "fsharp".to_string(),
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
        if ref_ctx.extracted_ref.chain.is_none() && !target.contains('.') {
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
                    strategy: "fsharp_synthetic_global",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        (DefaultResolver {
            file_ctx,
            ref_ctx,
            lookup,
            kind_compatible: predicates::kind_compatible,
        })
        .resolve_all()
    }
}

pub static FSHARP_HOOKS: FsharpHooks = FsharpHooks;
