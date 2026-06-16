// Swift language hooks. Absorbed from the deleted `swift/resolve.rs`.

use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct SwiftHooks;

pub(crate) fn detect_swift_vapor_route(
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
        CallArg::StringLit(s) => Some(s.as_str()),
        _ => None,
    })?;
    if url.is_empty() {
        return None;
    }
    let normalised = if url.starts_with('/') {
        url.to_string()
    } else {
        format!("/{}", url)
    };
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name: crate::connectors::url_pattern::normalize(&normalised),
        role: ChannelRole::Consumer,
        method: Some(method),
        streaming: None,
    })
}

pub(crate) fn detect_swift_http_chain(
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
    if leaf == "request" {
        let url = call_args.iter().find_map(|a| match a {
            CallArg::StringLit(s)
                if s.starts_with('/') || s.starts_with("http://") || s.starts_with("https://") =>
            {
                Some(s.as_str())
            }
            _ => None,
        })?;
        let root = chain.segments[0].name.as_str();
        if !matches!(root, "AF" | "Alamofire" | "session" | "URLSession") {
            return None;
        }
        return Some(FlowEmission::NamedChannel {
            kind: NamedChannelKind::HttpCall,
            name: crate::connectors::url_pattern::normalize(url),
            role: ChannelRole::Producer,
            method: Some(HttpMethod::Any),
            streaming: None,
        });
    }
    None
}

pub(crate) fn detect_swift_grdb_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    if chain.segments.len() < 2 {
        return None;
    }
    let root = chain.segments[0].name.as_str();
    let leaf = chain.segments.last()?.name.as_str();
    if !root
        .chars()
        .next()
        .map_or(false, |c| c.is_ascii_uppercase())
    {
        return None;
    }
    let op = match leaf {
        "fetchAll" | "fetchOne" | "fetchCursor" | "filter" | "order" | "select" | "all" => {
            DbQueryOp::Select
        }
        "insert" | "create" => DbQueryOp::Insert,
        "update" => DbQueryOp::Update,
        "delete" | "deleteAll" => DbQueryOp::Delete,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: format!("swift.{}", root),
        operation: op,
    })
}

pub(crate) fn detect_swift_grpc_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, NamedChannelKind, StreamKind,
    };
    if chain.segments.len() < 2 {
        return None;
    }
    let root = chain.segments[0].name.as_str();
    if !(root.ends_with("Client") || root.ends_with("AsyncClient")) || root == "Client" {
        return None;
    }
    let leaf = chain.segments.last()?.name.as_str();
    if matches!(leaf, "init") {
        return None;
    }
    let service = root
        .strip_suffix("AsyncClient")
        .or_else(|| root.strip_suffix("Client"))
        .unwrap_or(root);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name: format!("{}.{}", service, leaf),
        role: ChannelRole::Producer,
        method: None,
        streaming: Some(StreamKind::from_method_name(leaf)),
    })
}

fn manifest_dep_match(
    project_ctx: Option<&ProjectContext>,
    pkg_id: Option<i64>,
    root: &str,
) -> bool {
    let Some(ctx) = project_ctx else { return false };
    let Some(manifest) = ctx.manifests_for(pkg_id).get(&ManifestKind::SwiftPM) else {
        return false;
    };
    let root_lower = root.to_lowercase();
    manifest.dependencies.iter().any(|d| {
        let d_lower = d.to_lowercase();
        d_lower == root_lower || d_lower.trim_start_matches("swift-") == root_lower.as_str()
    })
}

#[cfg(test)]
pub(super) fn _test_manifest_dep_match(
    project_ctx: Option<&ProjectContext>,
    root: &str,
) -> bool {
    manifest_dep_match(project_ctx, None, root)
}

fn module_is_external(
    project_ctx: Option<&ProjectContext>,
    pkg_id: Option<i64>,
    lookup: Option<&dyn SymbolLookup>,
    root: &str,
) -> bool {
    if manifest_dep_match(project_ctx, pkg_id, root) {
        return true;
    }
    if predicates::is_external_swift_module(root) {
        return true;
    }
    if let Some(lookup) = lookup {
        if !lookup.has_in_namespace(root) {
            return true;
        }
    }
    false
}

pub(crate) fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
    lookup: Option<&dyn SymbolLookup>,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;
    let pkg_id = ref_ctx.file_package_id;
    if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
        let module = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
        let root = module.split('.').next().unwrap_or(module);
        if module_is_external(project_ctx, pkg_id, lookup, root) {
            return Some(root.to_string());
        }
        return None;
    }
    for import in &file_ctx.imports {
        let Some(module) = import.module_path.as_deref() else {
            continue;
        };
        if module.is_empty() {
            continue;
        }
        let root = module.split('.').next().unwrap_or(module);
        if module_is_external(project_ctx, pkg_id, lookup, root) {
            return Some(root.to_string());
        }
    }
    if target.contains('.') {
        let root = target.split('.').next().unwrap_or(target);
        if module_is_external(project_ctx, pkg_id, lookup, root) {
            return Some(root.to_string());
        }
    }
    None
}

pub(crate) fn detect_flow_inner(
    _file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    if r.chain.is_none() {
        if let Some(em) = detect_swift_vapor_route(r.target_name.as_str(), &r.call_args) {
            return vec![em];
        }
        return Vec::new();
    }
    let chain = r.chain.as_ref().unwrap();
    if let Some(em) = detect_swift_http_chain(chain, &r.call_args) {
        return vec![em];
    }
    if let Some(em) = detect_swift_grdb_emission(chain) {
        return vec![em];
    }
    if let Some(em) = detect_swift_grpc_emission(chain) {
        return vec![em];
    }
    Vec::new()
}

impl LanguageEngineHooks for SwiftHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_external_inner(file_ctx, ref_ctx, project_ctx, Some(lookup))
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
            let module = r.module.as_deref().unwrap_or(&r.target_name);
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module.to_string()),
                alias: None,
                is_wildcard: false,
            });
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "swift".to_string(),
            imports,
            file_namespace: None,
        })
    }
}

pub static SWIFT_HOOKS: SwiftHooks = SwiftHooks;
