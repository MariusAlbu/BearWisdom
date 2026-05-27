// Dart language hooks. Absorbed from the deleted `dart/resolve.rs`.

use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::chain::{self, identity_normalize, ChainConfig, NamespaceLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct DartHooks;

pub(crate) fn detect_dart_shelf_route(
    target_name: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    let method = match target_name {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
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

pub(crate) fn detect_dart_http_chain(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    if chain.segments.len() < 2 {
        return None;
    }
    let leaf = chain.segments.last()?.name.as_str();
    let method = match leaf {
        "get" => HttpMethod::Get,
        "post" => HttpMethod::Post,
        "put" => HttpMethod::Put,
        "patch" => HttpMethod::Patch,
        "delete" => HttpMethod::Delete,
        _ => return None,
    };
    let root = chain.segments[0].name.as_str();
    if !matches!(root, "dio" | "http" | "client" | "_client" | "Dio") {
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
        method: Some(method),
        streaming: None,
    })
}

pub(crate) fn detect_dart_drift_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    if chain.segments.len() < 2 {
        return None;
    }
    let root = chain.segments[0].name.as_str();
    let leaf = chain.segments.last()?.name.as_str();
    let op = match root {
        "select" => DbQueryOp::Select,
        "insert" => DbQueryOp::Insert,
        "update" => DbQueryOp::Update,
        "delete" => DbQueryOp::Delete,
        _ => return None,
    };
    if !matches!(
        leaf,
        "get" | "getSingle" | "watch" | "write" | "insert" | "go" | "go!" | "do"
    ) {
        return None;
    }
    Some(FlowEmission::DbQuery {
        entity_name: "dart.*".to_string(),
        operation: op,
    })
}

pub(crate) fn detect_dart_grpc_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    if chain.segments.len() < 2 {
        return None;
    }
    let root = chain.segments[0].name.as_str();
    if !root.ends_with("Client") || root == "Client" {
        return None;
    }
    let leaf = chain.segments.last()?.name.as_str();
    if matches!(leaf, "shutdown" | "terminate") {
        return None;
    }
    let service = root.strip_suffix("Client").unwrap_or(root);
    use crate::indexer::resolve::flow_emit::StreamKind;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name: format!("{}.{}", service, leaf),
        role: ChannelRole::Producer,
        method: None,
        streaming: Some(StreamKind::from_method_name(leaf)),
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
    if let Some(chain) = r.chain.as_ref() {
        if let Some(em) = detect_dart_http_chain(chain, &r.call_args) {
            return vec![em];
        }
        if let Some(em) = detect_dart_drift_emission(chain) {
            return vec![em];
        }
        if let Some(em) = detect_dart_grpc_emission(chain) {
            return vec![em];
        }
    }
    if let Some(em) = detect_dart_shelf_route(r.target_name.as_str(), &r.call_args) {
        return vec![em];
    }
    Vec::new()
}

impl LanguageEngineHooks for DartHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let uri = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            if predicates::is_external_dart_import(uri) {
                let ns = if uri.starts_with("dart:") {
                    "dart.stdlib"
                } else if let Some(pkg_path) = uri.strip_prefix("package:") {
                    pkg_path.split('/').next().unwrap_or(pkg_path)
                } else {
                    uri
                };
                return Some(ns.to_string());
            }
            if uri.starts_with("package:") {
                if let Some(ctx) = project_ctx {
                    if let Some(manifest) = ctx
                        .manifests_for(ref_ctx.file_package_id)
                        .get(&ManifestKind::Pubspec)
                    {
                        let pkg_name = uri
                            .strip_prefix("package:")
                            .unwrap_or(uri)
                            .split('/')
                            .next()
                            .unwrap_or(uri);
                        if manifest.dependencies.contains(pkg_name) {
                            return Some(pkg_name.to_string());
                        }
                    }
                }
            }
            return None;
        }
        let simple = target.split('.').next().unwrap_or(target);
        for import in &file_ctx.imports {
            let uri = import.module_path.as_deref().unwrap_or("");
            if uri.is_empty() {
                continue;
            }
            let pkg_name_from_uri = if uri.starts_with("package:") {
                uri.strip_prefix("package:")
                    .unwrap_or(uri)
                    .split('/')
                    .next()
                    .unwrap_or(uri)
            } else {
                ""
            };
            let is_manifest_external = !pkg_name_from_uri.is_empty()
                && project_ctx
                    .and_then(|ctx| {
                        ctx.manifests_for(ref_ctx.file_package_id)
                            .get(&ManifestKind::Pubspec)
                    })
                    .is_some_and(|m| m.dependencies.contains(pkg_name_from_uri));
            if let Some(alias) = &import.alias {
                if alias == simple
                    && (is_manifest_external || predicates::is_external_dart_import(uri))
                {
                    if uri.starts_with("package:") {
                        return Some(pkg_name_from_uri.to_string());
                    }
                    return Some(uri.to_string());
                }
            }
            if import.alias.is_none() {
                if is_manifest_external {
                    return Some(pkg_name_from_uri.to_string());
                }
                if predicates::is_external_dart_import(uri) {
                    if uri.starts_with("package:") {
                        return Some(pkg_name_from_uri.to_string());
                    }
                    return Some("dart.stdlib".to_string());
                }
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
            let uri = r.module.as_deref().unwrap_or(&r.target_name);
            let alias = if r.module.is_some() && r.target_name != uri {
                Some(r.target_name.clone())
            } else {
                None
            };
            imports.push(ImportEntry {
                imported_name: uri.to_string(),
                module_path: Some(uri.to_string()),
                alias,
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "dart".to_string(),
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
        if let Some(chain_val) = &ref_ctx.extracted_ref.chain {
            let config = ChainConfig {
                strategy_prefix: "dart",
                normalize_type: identity_normalize,
                has_self_ref: true,
                enclosing_type_kinds: &["class", "enum", "mixin"],
                static_type_kinds: &["class", "enum", "mixin", "type_alias", "extension"],
                use_generics: true,
                namespace_lookup: NamespaceLookup::None,
                kind_compatible: predicates::kind_compatible,
            };
            if let Some(res) = chain::resolve_via_chain(
                &config,
                chain_val,
                edge_kind,
                Some(file_ctx),
                ref_ctx,
                lookup,
            ) {
                return Some(res);
            }
        }
        let effective_target = target.strip_prefix("this.").unwrap_or(target);
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{effective_target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if predicates::kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "dart_scope_chain",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.name == effective_target
                && predicates::kind_compatible(edge_kind, &sym.kind)
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "dart_same_file",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        None
    }
}

pub static DART_HOOKS: DartHooks = DartHooks;
