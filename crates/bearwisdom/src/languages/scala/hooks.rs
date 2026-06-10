// =============================================================================
// languages/scala/hooks.rs — ScalaHooks impl of LanguageEngineHooks: external
// classification, flow detectors, and file-context (import + package)
// construction. Resolution runs through the generic engine; the chain walker
// expands `type` member aliases and qualifies bare receivers per
// `ChainQualification::SamePackageAndImports`.
// =============================================================================

use super::predicates;
use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

// sttp `basicRequest.get(uri"...")` / Akka HTTP `Http().singleRequest(...)`.
// Also covers WS Producer for `client.get("/x")`-style chains.
pub(crate) fn detect_scala_http_chain_emission(
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
        "head" => HttpMethod::Head,
        _ => return None,
    };
    let url = match call_args.first()? {
        CallArg::StringLit(s) => s.as_str(),
        _ => return None,
    };
    if !(url.starts_with('/') || url.starts_with("http://") || url.starts_with("https://")) {
        return None;
    }
    let name = crate::connectors::url_pattern::normalize(url);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Producer,
        method: Some(method),
        streaming: None,
    })
}

// http4s / Akka HTTP `path("users") { ... }`, route DSL — Consumer.
pub(crate) fn detect_scala_http_path_call(
    target_name: &str,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{
        ChannelRole, FlowEmission, HttpMethod, NamedChannelKind,
    };
    use crate::types::CallArg;
    if !matches!(target_name, "path" | "pathPrefix" | "pathEnd") {
        return None;
    }
    let url = match call_args.first()? {
        CallArg::StringLit(s) => s.as_str(),
        _ => return None,
    };
    let normalised = if url.starts_with('/') {
        url.to_string()
    } else {
        format!("/{}", url)
    };
    let name = crate::connectors::url_pattern::normalize(&normalised);
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::HttpCall,
        name,
        role: ChannelRole::Consumer,
        method: Some(HttpMethod::Any),
        streaming: None,
    })
}

// Slick `users.filter(_.id === id).result` — chain ends in `.result`,
// `.first`, etc. on a TableQuery.
pub(crate) fn detect_scala_db_query_emission(
    chain: &crate::types::MemberChain,
    _call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};

    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let root = segs[0].name.as_str();
    let leaf = segs.last()?.name.as_str();
    if !root
        .chars()
        .next()
        .map_or(false, |c| c.is_ascii_uppercase())
    {
        return None;
    }
    let op = match leaf {
        "result" | "list" | "headOption" | "to" | "filter" | "map" | "exists" => DbQueryOp::Select,
        "insert" | "+=" | "++=" | "insertOrUpdate" => DbQueryOp::Insert,
        "update" => DbQueryOp::Update,
        "delete" => DbQueryOp::Delete,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: format!("scala.{}", root),
        operation: op,
    })
}

// Doobie `sql"SELECT ... FROM users".query[User].option`. Chain has a leading
// `sql` segment (the interpolator); leaf op tells us the result-set verb.
pub(crate) fn detect_scala_doobie_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let segs = &chain.segments;
    if segs.len() < 2 {
        return None;
    }
    let has_sql_seg = segs.iter().any(|s| s.name == "sql");
    if !has_sql_seg {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    let op = match leaf {
        "query" | "option" | "unique" | "nel" | "to" | "stream" | "list" => DbQueryOp::Select,
        "update" | "run" => DbQueryOp::Update,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: "scala.doobie".to_string(),
        operation: op,
    })
}

// Quill `quote { query[User].filter(_.id == lift(id)) }`. The chain root or
// an inner segment is `query[T]` with an entity type parameter.
pub(crate) fn detect_scala_quill_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    let segs = &chain.segments;
    let query_seg = segs.iter().find(|s| s.name == "query")?;
    let entity = query_seg
        .type_args
        .first()
        .cloned()
        .unwrap_or_else(|| "*".to_string());
    let leaf = segs.last()?.name.as_str();
    let op = match leaf {
        "filter" | "map" | "sortBy" | "size" | "take" | "drop" | "groupBy" => DbQueryOp::Select,
        "insert" | "insertValue" => DbQueryOp::Insert,
        "update" | "updateValue" => DbQueryOp::Update,
        "delete" => DbQueryOp::Delete,
        _ => return None,
    };
    Some(FlowEmission::DbQuery {
        entity_name: format!("scala.{}", entity),
        operation: op,
    })
}

// ZIO SQL `select(*).from(users).where(...)`. The shape `.from(<Ident>)`
// names the table directly.
pub(crate) fn detect_scala_zio_sql_emission(
    chain: &crate::types::MemberChain,
    call_args: &[crate::types::CallArg],
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};
    use crate::types::CallArg;
    let leaf = chain.segments.last()?.name.as_str();
    if leaf != "from" {
        return None;
    }
    // Chain must contain a `select` segment earlier to avoid matching
    // arbitrary `foo.from(bar)` calls.
    if !chain.segments.iter().any(|s| s.name == "select") {
        return None;
    }
    let entity = call_args.iter().find_map(|a| match a {
        CallArg::Ident(name) if !name.is_empty() => Some(name.clone()),
        _ => None,
    })?;
    Some(FlowEmission::DbQuery {
        entity_name: format!("scala.{}", entity),
        operation: DbQueryOp::Select,
    })
}

pub(crate) fn detect_scala_grpc_emission(
    chain: &crate::types::MemberChain,
) -> Option<crate::indexer::resolve::flow_emit::FlowEmission> {
    use crate::indexer::resolve::flow_emit::{ChannelRole, FlowEmission, NamedChannelKind};
    let segs = &chain.segments;
    if segs.len() < 3 {
        return None;
    }
    let root = segs[0].name.as_str();
    if !root.ends_with("Grpc") && !root.ends_with("Client") {
        return None;
    }
    let ctor = segs[1].name.as_str();
    if !matches!(
        ctor,
        "stub" | "blockingStub" | "newStub" | "newBlockingStub" | "apply"
    ) {
        return None;
    }
    let leaf = segs.last()?.name.as_str();
    if matches!(
        leaf,
        "stub" | "blockingStub" | "newStub" | "newBlockingStub" | "apply"
    ) {
        return None;
    }
    let service = root
        .strip_suffix("Grpc")
        .or_else(|| root.strip_suffix("Client"))
        .unwrap_or(root);
    use crate::indexer::resolve::flow_emit::StreamKind;
    Some(FlowEmission::NamedChannel {
        kind: NamedChannelKind::RpcCall,
        name: format!("{}.{}", service, leaf),
        role: ChannelRole::Producer,
        method: None,
        streaming: Some(StreamKind::from_method_name(leaf)),
    })
}

pub(crate) fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    project_ctx: Option<&ProjectContext>,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;

    if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
        let import_path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);

        if let Some(ctx) = project_ctx {
            for kind in [ManifestKind::Maven, ManifestKind::Gradle] {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&kind) {
                    if manifest.dependencies.iter().any(|group_id| {
                        import_path == group_id
                            || import_path.starts_with(group_id.as_str())
                                && import_path.as_bytes().get(group_id.len()) == Some(&b'.')
                    }) {
                        return Some(import_path.to_string());
                    }
                }
            }
        }

        if predicates::is_external_scala_namespace(import_path, project_ctx) {
            return Some(import_path.to_string());
        }
        return None;
    }

    for import in &file_ctx.imports {
        let ns = import.module_path.as_deref().unwrap_or("");
        if ns.is_empty() {
            continue;
        }
        if !import.is_wildcard
            && import.imported_name != *target
            && import.alias.as_deref() != Some(target.as_str())
        {
            continue;
        }

        if let Some(ctx) = project_ctx {
            for kind in [ManifestKind::Maven, ManifestKind::Gradle] {
                if let Some(manifest) = ctx.manifests_for(ref_ctx.file_package_id).get(&kind) {
                    if manifest.dependencies.iter().any(|group_id| {
                        ns == group_id
                            || ns.starts_with(group_id.as_str())
                                && ns.as_bytes().get(group_id.len()) == Some(&b'.')
                    }) {
                        return Some(ns.to_string());
                    }
                }
            }
        }

        if predicates::is_external_scala_namespace(ns, project_ctx) {
            return Some(ns.to_string());
        }
    }

    if predicates::effective_target_is_external(target, project_ctx) {
        return Some(target.clone());
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
    if let Some(chain) = r.chain.as_ref() {
        if let Some(em) = detect_scala_http_chain_emission(chain, &r.call_args) {
            return vec![em];
        }
        if let Some(em) = detect_scala_db_query_emission(chain, &r.call_args) {
            return vec![em];
        }
        if let Some(em) = detect_scala_doobie_emission(chain) {
            return vec![em];
        }
        if let Some(em) = detect_scala_quill_emission(chain) {
            return vec![em];
        }
        if let Some(em) = detect_scala_zio_sql_emission(chain, &r.call_args) {
            return vec![em];
        }
        if let Some(em) = detect_scala_grpc_emission(chain) {
            return vec![em];
        }
    }
    // http4s / Akka HTTP `path("x") { get { ... } }` — bare calls.
    if let Some(em) = detect_scala_http_path_call(r.target_name.as_str(), &r.call_args) {
        return vec![em];
    }
    Vec::new()
}

pub(crate) fn build_file_context_inner(
    file: &ParsedFile,
    _project_ctx: Option<&ProjectContext>,
) -> FileContext {
    let mut imports = Vec::new();

    let file_namespace = file.symbols.iter().find_map(|sym| {
        if sym.kind == crate::types::SymbolKind::Namespace {
            Some(sym.qualified_name.clone())
        } else {
            None
        }
    });

    for r in &file.refs {
        if r.kind != EdgeKind::Imports {
            continue;
        }
        let module = r.module.as_deref().unwrap_or(&r.target_name);
        // Scala wildcard is `_` (Scala 2) or `*` (Scala 3).
        let is_wildcard = r.target_name == "_" || r.target_name == "*";

        if is_wildcard {
            imports.push(ImportEntry {
                imported_name: String::new(),
                module_path: Some(module.to_string()),
                alias: None,
                is_wildcard: true,
            });
        } else {
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(module.to_string()),
                alias: None,
                is_wildcard: false,
            });
        }
    }

    FileContext {
        file_path: file.path.clone(),
        language: "scala".to_string(),
        imports,
        file_namespace,
    }
}

pub struct ScalaHooks;

impl LanguageEngineHooks for ScalaHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        let _ = lookup;
        infer_external_inner(file_ctx, ref_ctx, project_ctx)
    }

    fn detect_flow_emissions(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Vec<crate::indexer::resolve::flow_emit::FlowEmission> {
        let _ = lookup;
        detect_flow_inner(file_ctx, ref_ctx)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(build_file_context_inner(file, project_ctx))
    }
}

pub static SCALA_HOOKS: ScalaHooks = ScalaHooks;
