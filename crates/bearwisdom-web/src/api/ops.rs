use std::path::{Path, PathBuf};

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::api::{
    default_depth_3, default_forward, default_limit_500, default_max_nodes, err_json, ok_json,
    DeadCodeQuery, IndexBody, PathParam,
};
use crate::db::{db_exists, resolve_db_path};
use crate::AppState;

// ---------------------------------------------------------------------------
// POST /api/index
// ---------------------------------------------------------------------------

pub async fn post_index(
    State(state): State<AppState>,
    Json(body): Json<IndexBody>,
) -> impl IntoResponse {
    let root = PathBuf::from(&body.path);
    match index_project(&root, &state.pool) {
        Ok(stats) => ok_json(stats).into_response(),
        Err(e) => err_json(e).into_response(),
    }
}

fn index_project(
    root: &Path,
    pool_state: &crate::db::PoolState,
) -> anyhow::Result<serde_json::Value> {
    let db_path = resolve_db_path(root)?;
    let already_existed = db_exists(root);

    let mut db = bearwisdom::Database::open(&db_path)?;

    if already_existed {
        // DB exists — read stats without re-indexing, then register pool.
        let s = bearwisdom::query::stats::index_stats(&db)?;

        // Register a pool so subsequent GET handlers use pooled connections.
        drop(db);
        let pool = bearwisdom::DbPool::new(&db_path, 4)?;
        pool_state.set_pool(root, pool);

        return Ok(json!({
            "file_count": s.file_count,
            "symbol_count": s.symbol_count,
            "edge_count": s.edge_count,
            "unresolved_ref_count": s.unresolved_ref_count,
            "unresolved_ref_count_external": s.unresolved_ref_count_external,
            "external_ref_count": s.external_ref_count,
            "duration_ms": 0,
            "cached": true,
        }));
    }

    let stats = bearwisdom::full_index(&mut db, root, None, None, None)?;
    let _ = bearwisdom::query::concepts::discover_concepts(&db);
    let _ = bearwisdom::query::concepts::auto_assign_concepts(&db);

    // If no concepts were discovered (flat qualified names), create directory-based concepts.
    if bearwisdom::query::stats::concept_count(&db)? == 0 {
        let _ = bearwisdom::query::concepts::discover_directory_concepts(&db);
    }

    // Embedding runs separately — too slow to block the index response.
    // Use `bw embed` CLI or the hybrid search will embed on first query.

    // Register a pool for subsequent GET handlers.
    drop(db);
    let pool = bearwisdom::DbPool::new(&db_path, 4)?;
    pool_state.set_pool(root, pool);

    Ok(json!({
        "file_count": stats.file_count,
        "symbol_count": stats.symbol_count,
        "edge_count": stats.edge_count,
        "unresolved_ref_count": stats.unresolved_ref_count,
        "unresolved_ref_count_external": stats.unresolved_ref_count_external,
        "external_ref_count": stats.external_ref_count,
        "duration_ms": stats.duration_ms,
        "cached": false,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/status
// ---------------------------------------------------------------------------

pub async fn get_status(
    State(state): State<AppState>,
    Query(params): Query<PathParam>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match status_counts(&root, &state.pool) {
        Ok(v) => ok_json(v).into_response(),
        Err(e) => err_json(e).into_response(),
    }
}

fn status_counts(
    root: &Path,
    pool_state: &crate::db::PoolState,
) -> anyhow::Result<serde_json::Value> {
    let db = pool_state.get_db(root)?;
    let s = bearwisdom::query::stats::index_stats(&db)?;
    Ok(json!({
        "file_count": s.file_count,
        "symbol_count": s.symbol_count,
        "edge_count": s.edge_count,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/architecture
// ---------------------------------------------------------------------------

pub async fn get_architecture(
    State(state): State<AppState>,
    Query(params): Query<PathParam>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => match bearwisdom::query::architecture::get_overview(&db) {
            Ok(overview) => ok_json(overview).into_response(),
            Err(e) => err_json(e).into_response(),
        },
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/dead-code
// ---------------------------------------------------------------------------

pub async fn get_dead_code(
    State(state): State<AppState>,
    Query(params): Query<DeadCodeQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    let vis = match params.visibility.as_str() {
        "private" => bearwisdom::query::dead_code::VisibilityFilter::PrivateOnly,
        "public" => bearwisdom::query::dead_code::VisibilityFilter::PublicOnly,
        _ => bearwisdom::query::dead_code::VisibilityFilter::All,
    };
    let options = bearwisdom::query::dead_code::DeadCodeOptions {
        scope: params.scope,
        visibility_filter: vis,
        include_tests: params.include_tests,
        max_results: params.limit,
        ..Default::default()
    };
    match state.pool.get_db(&root) {
        Ok(db) => match bearwisdom::query::dead_code::find_dead_code(&db, &options) {
            Ok(report) => ok_json(report).into_response(),
            Err(e) => err_json(e).into_response(),
        },
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/entry-points
// ---------------------------------------------------------------------------

pub async fn get_entry_points(
    State(state): State<AppState>,
    Query(params): Query<PathParam>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => match bearwisdom::query::dead_code::find_entry_points(&db) {
            Ok(report) => ok_json(report).into_response(),
            Err(e) => err_json(e).into_response(),
        },
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// POST /api/embed
// ---------------------------------------------------------------------------

pub async fn post_embed(
    State(state): State<AppState>,
    Json(body): Json<IndexBody>,
) -> impl IntoResponse {
    // Reject if already running.
    {
        let s = state.embed_status.lock().unwrap();
        if s.state == "running" {
            return ok_json(json!({"started": false, "reason": "already running"})).into_response();
        }
    }

    // Mark as running immediately.
    {
        let mut s = state.embed_status.lock().unwrap();
        s.state = "running";
        s.embedded = 0;
        s.error = None;
    }

    let root = PathBuf::from(&body.path);
    let bg_status = state.embed_status.clone();

    // Run embedding in a background thread — don't block the request.
    tokio::task::spawn_blocking(move || {
        let result = (|| -> anyhow::Result<u32> {
            let db_path = resolve_db_path(&root)?;
            let db = bearwisdom::Database::open(&db_path)?;
            let model_dir = bearwisdom::search::embedder::Embedder::resolve_model_dir(&root)
                .ok_or_else(|| anyhow::anyhow!("No CodeRankEmbed model found"))?;
            let mut embedder = bearwisdom::search::embedder::Embedder::new(model_dir);
            let (n, _) = bearwisdom::embed_chunks(&db, &mut embedder, 4)?;
            embedder.unload();
            Ok(n)
        })();

        let mut s = bg_status.lock().unwrap();
        match result {
            Ok(n) => {
                s.state = "done";
                s.embedded = n;
                tracing::info!("Embedding complete: {n} chunks");
            }
            Err(e) => {
                s.state = "error";
                s.error = Some(format!("{e:#}"));
                tracing::warn!("Embedding failed: {e:#}");
            }
        }
    });

    ok_json(json!({"started": true})).into_response()
}

// ---------------------------------------------------------------------------
// GET /api/embed-status
// ---------------------------------------------------------------------------

pub async fn get_embed_status(State(state): State<AppState>) -> impl IntoResponse {
    let s = state.embed_status.lock().unwrap();
    ok_json(json!({
        "state": s.state,
        "embedded": s.embedded,
        "error": s.error,
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// GET /api/flow-edges
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct FlowEdgesQuery {
    path: String,
    #[serde(default = "default_limit_500")]
    limit: usize,
}

pub async fn get_flow_edges(
    State(state): State<AppState>,
    Query(params): Query<FlowEdgesQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => match query_flow_edges(&db, params.limit) {
            Ok(v) => ok_json(v).into_response(),
            Err(e) => err_json(e).into_response(),
        },
        Err(e) => err_json(e).into_response(),
    }
}

fn query_flow_edges(db: &bearwisdom::Database, limit: usize) -> anyhow::Result<serde_json::Value> {
    let d = bearwisdom::query::stats::flow_edges_data(db, limit)?;
    Ok(json!({
        "edges": d.edges,
        "summary": {
            "total": d.total,
            "by_edge_type": d.by_edge_type,
            "by_language_pair": d.by_language_pair,
        }
    }))
}

// ---------------------------------------------------------------------------
// GET /api/trace-flow
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct TraceFlowQuery {
    path: String,
    file: String,
    #[serde(default)]
    line: u32,
    #[serde(default = "default_depth_3")]
    depth: u32,
    #[serde(default = "default_forward")]
    direction: String,
}

pub async fn get_trace_flow(
    State(state): State<AppState>,
    Query(params): Query<TraceFlowQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => {
            let result = match params.direction.as_str() {
                "backward" => bearwisdom::search::flow::trace_flow_reverse(
                    &db,
                    &params.file,
                    params.line,
                    params.depth,
                ),
                "both" => bearwisdom::search::flow::trace_flow_bidirectional(
                    &db,
                    &params.file,
                    params.line,
                    params.depth,
                )
                .map(|b| {
                    let mut steps = b.forward;
                    steps.extend(b.backward);
                    steps
                }),
                _ => bearwisdom::search::flow::trace_flow(
                    &db,
                    &params.file,
                    params.line,
                    params.depth,
                ),
            };
            match result {
                Ok(steps) => ok_json(steps).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/full-trace
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct FullTraceQuery {
    path: String,
    symbol: Option<String>,
    #[serde(default = "default_depth_3")]
    depth: u32,
    #[serde(default = "default_max_traces")]
    max_traces: usize,
}

fn default_max_traces() -> usize {
    15
}

pub async fn get_full_trace(
    State(state): State<AppState>,
    Query(params): Query<FullTraceQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => {
            let result = match params.symbol.as_deref() {
                Some(sym) => {
                    bearwisdom::query::full_trace::trace_from_symbol(&db, sym, params.depth)
                }
                None => bearwisdom::query::full_trace::trace_from_entry_points(
                    &db,
                    params.depth,
                    params.max_traces,
                ),
            };
            match result {
                Ok(r) => ok_json(r).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/hierarchy
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct HierarchyQuery {
    path: String,
    /// "services", "packages", "files", or "symbols". Defaults to "packages".
    #[serde(default = "default_level_packages")]
    level: String,
    /// Package path (for "files" level) or file path (for "symbols" level).
    scope: Option<String>,
    /// Maximum nodes to return (default: 500).
    #[serde(default = "default_max_nodes")]
    max_nodes: usize,
}

fn default_level_packages() -> String {
    "packages".to_string()
}

pub async fn get_hierarchy(
    State(state): State<AppState>,
    Query(params): Query<HierarchyQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => {
            let scope = params.scope.as_deref().filter(|s| !s.is_empty());
            match bearwisdom::hierarchical_graph(&db, &params.level, scope, params.max_nodes) {
                Ok(result) => ok_json(result).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/packages
// ---------------------------------------------------------------------------

pub async fn get_packages(
    State(state): State<AppState>,
    Query(params): Query<PathParam>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => match bearwisdom::list_packages(&db) {
            Ok(packages) => ok_json(packages).into_response(),
            Err(e) => err_json(e).into_response(),
        },
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/workspace
// ---------------------------------------------------------------------------

pub async fn get_workspace(
    State(state): State<AppState>,
    Query(params): Query<PathParam>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => match bearwisdom::workspace_overview(&db) {
            Ok(overview) => ok_json(overview).into_response(),
            Err(e) => err_json(e).into_response(),
        },
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/dependencies
// ---------------------------------------------------------------------------

pub async fn get_dependencies(
    State(state): State<AppState>,
    Query(params): Query<PathParam>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => match bearwisdom::package_dependencies(&db) {
            Ok(deps) => ok_json(deps).into_response(),
            Err(e) => err_json(e).into_response(),
        },
        Err(e) => err_json(e).into_response(),
    }
}
