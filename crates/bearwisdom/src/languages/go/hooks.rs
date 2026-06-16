// =============================================================================
// languages/go/hooks.rs — GoHooks impl of LanguageEngineHooks: external
// classification, flow-emission detection (HTTP / DB / gRPC / mailer / bgjob /
// MQ / Redis / UDS / Gorilla WS) with let-binding gRPC propagation, and
// file-context (import + package) construction.
//
// Resolution runs through the generic engine. Go's package-qualified calls
// (`gin.NewRouter()`) drop the qualifier in the extractor — only the
// field_identifier lands in target_name; members are keyed under the import's
// package short name (`gin.NewRouter`). The chain walker / DefaultResolver
// bridge that via `ChainQualification::PackageShortName`.
// =============================================================================

pub(crate) use super::flow_detectors::{
    detect_go_bgjob_emission, detect_go_db_query_emission, detect_go_gorilla_ws_consumer,
    detect_go_grpc_chain_emission, detect_go_http_chain_emission, detect_go_mailer_emission,
    detect_go_mq_emission, detect_go_redis_config_lookup, detect_go_uds_emission,
};
use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

fn extract_package_name(file: &ParsedFile) -> Option<String> {
    for sym in &file.symbols {
        if let Some(dot) = sym.qualified_name.find('.') {
            let pkg = &sym.qualified_name[..dot];
            if !pkg.is_empty() {
                return Some(pkg.to_string());
            }
        }
        if let Some(ref sp) = sym.scope_path {
            if !sp.is_empty() {
                return Some(sp.split('.').next().unwrap_or(sp.as_str()).to_string());
            }
        }
    }
    None
}

// Returns false when the path matches (or is a sub-package of) the project's
// own module path. Falls back to dot-in-first-segment when no GoMod manifest.
pub(crate) fn is_manifest_go_external(ctx: &ProjectContext, import_path: &str) -> bool {
    let module_path: Option<&str> = ctx
        .manifest(ManifestKind::GoMod)
        .and_then(|m| m.module_path.as_deref());

    if let Some(module_path) = module_path {
        if import_path == module_path {
            return false;
        }
        if import_path.starts_with(module_path)
            && import_path.len() > module_path.len()
            && import_path.as_bytes()[module_path.len()] == b'/'
        {
            return false;
        }
        return true;
    }
    let first_segment = import_path.split('/').next().unwrap_or(import_path);
    first_segment.contains('.')
}

pub(crate) fn detect_flow_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let r = &ref_ctx.extracted_ref;
    if r.kind != EdgeKind::Calls {
        return Vec::new();
    }
    let Some(chain) = r.chain.as_ref() else {
        return Vec::new();
    };
    if let Some(emission) = detect_go_http_chain_emission(chain, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_go_db_query_emission(chain, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_go_grpc_chain_emission(chain, file_ctx) {
        return vec![emission];
    }
    if let Some(emission) = detect_go_mailer_emission(chain, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_go_bgjob_emission(chain) {
        return vec![emission];
    }
    if let Some(emission) = detect_go_mq_emission(chain, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_go_redis_config_lookup(chain, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_go_uds_emission(chain, &r.call_args) {
        return vec![emission];
    }
    if let Some(emission) = detect_go_gorilla_ws_consumer(chain) {
        return vec![emission];
    }
    Vec::new()
}

pub(crate) fn detect_flow_inner_with_lookup(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
    let direct = detect_flow_inner(file_ctx, ref_ctx);
    if !direct.is_empty() {
        return direct;
    }
    // Let-binding propagation: `client := userpb.NewUserServiceClient(conn)`
    // followed by `client.GetUser(...)`. The variable's recorded type
    // comes from the Go extractor's TypeRef on the Variable symbol.
    let r = &ref_ctx.extracted_ref;
    let Some(chain) = r.chain.as_ref() else {
        return Vec::new();
    };
    let Some(root_seg) = chain.segments.first() else {
        return Vec::new();
    };
    if !matches!(root_seg.kind, crate::types::SegmentKind::Identifier) {
        return Vec::new();
    }
    let var_qname = match ref_ctx.source_symbol.scope_path.as_deref() {
        Some(scope) => format!("{}.{}", scope, root_seg.name),
        None => root_seg.name.clone(),
    };
    let type_name = match lookup.field_type_str(&var_qname) {
        Some(t) => t.to_string(),
        None => return Vec::new(),
    };
    if !type_name.ends_with("Client") {
        return Vec::new();
    }
    let mut new_segments = vec![crate::types::ChainSegment {
        name: type_name,
        node_kind: "rewritten_var".to_string(),
        kind: crate::types::SegmentKind::Identifier,
        declared_type: None,
        type_args: vec![],
        optional_chaining: false,
        byte_offset: 0,
        declared_type_id: None,
        is_call: false,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    }];
    new_segments.extend(chain.segments.iter().skip(1).cloned());
    let rewritten = crate::types::MemberChain {
        segments: new_segments,
    };
    if let Some(em) = detect_go_grpc_chain_emission(&rewritten, file_ctx) {
        return vec![em];
    }
    Vec::new()
}

pub(crate) fn build_file_context_inner(
    file: &ParsedFile,
    _project_ctx: Option<&ProjectContext>,
) -> FileContext {
    let mut imports = Vec::new();

    let file_namespace = extract_package_name(file);

    for r in &file.refs {
        if r.kind != EdgeKind::Imports {
            continue;
        }
        let full_path = match &r.module {
            Some(m) => m.clone(),
            None => r.target_name.clone(),
        };

        let last_segment = full_path.rsplit('/').next().unwrap_or(&full_path);

        // Blank import (`import _ "path"`) — side effects only.
        if r.target_name == "_" {
            continue;
        }

        // Dot import (`import . "path"`) — exported names enter scope directly.
        let is_dot_import = r.target_name == ".";

        let alias = if is_dot_import || r.target_name == last_segment {
            None
        } else {
            Some(r.target_name.clone())
        };

        imports.push(ImportEntry {
            imported_name: alias.clone().unwrap_or_else(|| last_segment.to_string()),
            module_path: Some(full_path),
            alias,
            is_wildcard: is_dot_import,
        });
    }

    FileContext {
        file_path: file.path.clone(),
        language: "go".to_string(),
        imports,
        file_namespace,
    }
}

pub struct GoHooks;

impl LanguageEngineHooks for GoHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            return Some(import_path.to_string());
        }

        if predicates::is_go_builtin(target) || predicates::is_go_composite_type(target) {
            return Some("builtin".to_string());
        }

        let is_exported = target.chars().next().is_some_and(|c| c.is_uppercase());
        if !is_exported {
            return None;
        }

        // Prefer the longest matching external module path. Manifest-driven
        // via go.mod when ProjectContext is available.
        let mut best: Option<&str> = None;
        for import in &file_ctx.imports {
            let Some(full_path) = &import.module_path else {
                continue;
            };
            let external = if let Some(ctx) = project_ctx {
                is_manifest_go_external(ctx, full_path)
            } else {
                predicates::is_external_go_import_fallback(full_path)
            };
            if external && (best.is_none() || full_path.len() > best.unwrap().len()) {
                best = Some(full_path.as_str());
            }
        }
        best.map(|s| s.to_string())
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        detect_flow_inner_with_lookup(file_ctx, ref_ctx, lookup)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(build_file_context_inner(file, project_ctx))
    }
}

pub static GO_HOOKS: GoHooks = GoHooks;
