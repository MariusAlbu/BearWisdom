use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::db::{db_exists, resolve_db_path};
use crate::AppState;

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

fn ok_json<T: serde::Serialize>(data: T) -> impl IntoResponse {
    Json(json!({"ok": true, "data": data}))
}

fn err_json(msg: impl std::fmt::Display) -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"ok": false, "error": msg.to_string()})),
    )
}

// ---------------------------------------------------------------------------
// Common query parameter structs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct PathParam {
    path: String,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    path: String,
    q: Option<String>,
    #[serde(default = "default_limit_20")]
    limit: usize,
}

#[derive(Deserialize)]
pub struct SymbolQuery {
    path: String,
    symbol: String,
}

#[derive(Deserialize)]
pub struct SymbolLimitQuery {
    path: String,
    symbol: String,
    #[serde(default = "default_limit_100")]
    limit: usize,
}

#[derive(Deserialize)]
pub struct BlastQuery {
    path: String,
    symbol: String,
    #[serde(default = "default_depth_3")]
    depth: u32,
}

#[derive(Deserialize)]
pub struct FileQuery {
    path: String,
    file: String,
}

#[derive(Deserialize)]
pub struct GraphQuery {
    path: String,
    filter: Option<String>,
    #[serde(default = "default_max_nodes")]
    max_nodes: usize,
}

#[derive(Deserialize)]
pub struct ConceptMembersQuery {
    path: String,
    concept: String,
    #[serde(default = "default_limit_100")]
    limit: usize,
}

#[derive(Deserialize)]
pub struct GrepQuery {
    path: String,
    pattern: String,
    #[serde(default)]
    regex: bool,
    #[serde(default = "default_true")]
    case_insensitive: bool,
    #[serde(default = "default_limit_200")]
    limit: usize,
}

#[derive(Deserialize)]
pub struct BrowseQuery {
    path: Option<String>,
}

#[derive(Deserialize)]
pub struct DeadCodeQuery {
    path: String,
    scope: Option<String>,
    #[serde(default = "default_visibility_all")]
    visibility: String,
    #[serde(default)]
    include_tests: bool,
    #[serde(default = "default_limit_100")]
    limit: usize,
}

fn default_visibility_all() -> String { "all".to_string() }

fn default_limit_20() -> usize { 20 }
fn default_limit_100() -> usize { 100 }
fn default_limit_200() -> usize { 200 }
fn default_limit_500() -> usize { 500 }
fn default_depth_3() -> u32 { 3 }
fn default_max_nodes() -> usize { 500 }
fn default_true() -> bool { true }
fn default_forward() -> String { "forward".to_string() }

// ---------------------------------------------------------------------------
// POST /api/index
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct IndexBody {
    path: String,
}

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

fn index_project(root: &Path, pool_state: &crate::db::PoolState) -> anyhow::Result<serde_json::Value> {
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

fn status_counts(root: &Path, pool_state: &crate::db::PoolState) -> anyhow::Result<serde_json::Value> {
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
// GET /api/search-symbols
// ---------------------------------------------------------------------------

pub async fn get_search_symbols(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    let query = params.q.unwrap_or_default();
    match state.pool.get_db(&root) {
        Ok(db) => {
            match bearwisdom::query::search::search_symbols(&db, &query, params.limit, &bearwisdom::query::QueryOptions::full()) {
                Ok(results) => ok_json(results).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/fuzzy-files
// ---------------------------------------------------------------------------

pub async fn get_fuzzy_files(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    let query = params.q.unwrap_or_default();
    match state.pool.get_db(&root) {
        Ok(db) => {
            match bearwisdom::search::fuzzy::FuzzyIndex::from_db(&db) {
                Ok(idx) => ok_json(idx.match_files(&query, params.limit)).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/fuzzy-symbols
// ---------------------------------------------------------------------------

pub async fn get_fuzzy_symbols(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    let query = params.q.unwrap_or_default();
    match state.pool.get_db(&root) {
        Ok(db) => {
            match bearwisdom::search::fuzzy::FuzzyIndex::from_db(&db) {
                Ok(idx) => ok_json(idx.match_symbols(&query, params.limit)).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/grep
// ---------------------------------------------------------------------------

pub async fn get_grep(Query(params): Query<GrepQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    let cancel = Arc::new(AtomicBool::new(false));
    let options = bearwisdom::search::grep::GrepOptions {
        case_sensitive: !params.case_insensitive,
        regex: params.regex,
        max_results: params.limit,
        ..Default::default()
    };
    match bearwisdom::search::grep::grep_search(&root, &params.pattern, &options, &cancel) {
        Ok(results) => ok_json(results).into_response(),
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/search-content
// ---------------------------------------------------------------------------

pub async fn get_search_content(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => {
            let scope = bearwisdom::search::scope::SearchScope::default();
            let query = params.q.as_deref().unwrap_or("");
            match bearwisdom::search::content_search::search_content(&db, query, &scope, params.limit) {
                Ok(results) => ok_json(results).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/hybrid
// ---------------------------------------------------------------------------

pub async fn get_hybrid(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => {
            let model_dir = bearwisdom::search::embedder::Embedder::resolve_model_dir(&root)
                .unwrap_or_else(|| root.join("models").join("CodeRankEmbed"));
            let mut embedder = bearwisdom::search::embedder::Embedder::new(model_dir);
            let scope = bearwisdom::search::scope::SearchScope::default();
            let query = params.q.as_deref().unwrap_or("");
            match bearwisdom::search::hybrid::hybrid_search(&db, &mut embedder, query, &scope, params.limit) {
                Ok(results) => ok_json(results).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/graph
// ---------------------------------------------------------------------------

pub async fn get_graph(
    State(state): State<AppState>,
    Query(params): Query<GraphQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    let filter = params.filter.as_deref().filter(|f| !f.is_empty());
    match state.pool.get_db(&root) {
        Ok(db) => {
            match bearwisdom::query::subgraph::export_graph(&db, filter, params.max_nodes) {
                Ok(result) => ok_json(result).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/concepts
// ---------------------------------------------------------------------------

pub async fn get_concepts(
    State(state): State<AppState>,
    Query(params): Query<PathParam>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => match bearwisdom::query::concepts::list_concepts(&db) {
            Ok(concepts) => ok_json(concepts).into_response(),
            Err(e) => err_json(e).into_response(),
        },
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/concept-members
// ---------------------------------------------------------------------------

pub async fn get_concept_members(
    State(state): State<AppState>,
    Query(params): Query<ConceptMembersQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => {
            match bearwisdom::query::concepts::concept_members(
                &db,
                &params.concept,
                params.limit,
            ) {
                Ok(members) => ok_json(members).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/symbol-info
// ---------------------------------------------------------------------------

pub async fn get_symbol_info(
    State(state): State<AppState>,
    Query(params): Query<SymbolQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => match bearwisdom::query::symbol_info::symbol_info(&db, &params.symbol, &bearwisdom::query::QueryOptions::full()) {
            Ok(info) => ok_json(info).into_response(),
            Err(e) => err_json(e).into_response(),
        },
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/definition
// ---------------------------------------------------------------------------

pub async fn get_definition(
    State(state): State<AppState>,
    Query(params): Query<SymbolQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => {
            match bearwisdom::query::definitions::goto_definition(&db, &params.symbol) {
                Ok(defs) => ok_json(defs).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/references
// ---------------------------------------------------------------------------

pub async fn get_references(
    State(state): State<AppState>,
    Query(params): Query<SymbolLimitQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => {
            match bearwisdom::query::references::find_references(
                &db,
                &params.symbol,
                params.limit,
            ) {
                Ok(refs) => ok_json(refs).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/calls-in
// ---------------------------------------------------------------------------

pub async fn get_calls_in(
    State(state): State<AppState>,
    Query(params): Query<SymbolLimitQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => {
            match bearwisdom::query::call_hierarchy::incoming_calls(
                &db,
                &params.symbol,
                params.limit,
            ) {
                Ok(items) => ok_json(items).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/calls-out
// ---------------------------------------------------------------------------

pub async fn get_calls_out(
    State(state): State<AppState>,
    Query(params): Query<SymbolLimitQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => {
            match bearwisdom::query::call_hierarchy::outgoing_calls(
                &db,
                &params.symbol,
                params.limit,
            ) {
                Ok(items) => ok_json(items).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/blast-radius
// ---------------------------------------------------------------------------

pub async fn get_blast_radius(
    State(state): State<AppState>,
    Query(params): Query<BlastQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => {
            match bearwisdom::query::blast_radius::blast_radius(
                &db,
                &params.symbol,
                params.depth,
                500,
            ) {
                Ok(result) => ok_json(result).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/file-symbols
// ---------------------------------------------------------------------------

pub async fn get_file_symbols(
    State(state): State<AppState>,
    Query(params): Query<FileQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match file_symbols(&root, &params.file, &state.pool) {
        Ok(v) => ok_json(v).into_response(),
        Err(e) => err_json(e).into_response(),
    }
}

fn file_symbols(
    root: &Path,
    file: &str,
    pool_state: &crate::db::PoolState,
) -> anyhow::Result<Vec<bearwisdom::FileSymbol>> {
    let db = pool_state.get_db(root)?;
    Ok(bearwisdom::query::symbol_info::file_symbols(&db, file, bearwisdom::FileSymbolsMode::Full)?)
}

// ---------------------------------------------------------------------------
// GET /api/file-content
// ---------------------------------------------------------------------------

pub async fn get_file_content(Query(params): Query<FileQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    let full_path = root.join(&params.file);
    match std::fs::read_to_string(&full_path) {
        Ok(content) => ok_json(json!({"content": content})).into_response(),
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/browse
// ---------------------------------------------------------------------------

pub async fn get_browse(Query(params): Query<BrowseQuery>) -> impl IntoResponse {
    let path_str = params.path.unwrap_or_default();

    // On Windows with an empty path, enumerate drive letters.
    #[cfg(windows)]
    if path_str.is_empty() {
        let drives = list_windows_drives();
        return ok_json(json!({
            "dirs": drives,
            "files": serde_json::Value::Array(vec![]),
        }))
        .into_response();
    }

    let dir = if path_str.is_empty() {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
    } else {
        PathBuf::from(&path_str)
    };

    match browse_dir(&dir) {
        Ok((dirs_list, files_list)) => {
            ok_json(json!({"dirs": dirs_list, "files": files_list})).into_response()
        }
        Err(e) => err_json(e).into_response(),
    }
}

fn browse_dir(dir: &Path) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let mut dirs_out: Vec<String> = Vec::new();
    let mut files_out: Vec<String> = Vec::new();

    for entry in std::fs::read_dir(dir)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => dirs_out.push(name),
            Ok(ft) if ft.is_file() => files_out.push(name),
            _ => {}
        }
    }

    dirs_out.sort();
    files_out.sort();
    Ok((dirs_out, files_out))
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
                    &db, &params.file, params.line, params.depth,
                ),
                "both" => bearwisdom::search::flow::trace_flow_bidirectional(
                    &db, &params.file, params.line, params.depth,
                )
                .map(|b| {
                    let mut steps = b.forward;
                    steps.extend(b.backward);
                    steps
                }),
                _ => bearwisdom::search::flow::trace_flow(
                    &db, &params.file, params.line, params.depth,
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

fn default_max_traces() -> usize { 15 }

pub async fn get_full_trace(
    State(state): State<AppState>,
    Query(params): Query<FullTraceQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => {
            let result = match params.symbol.as_deref() {
                Some(sym) => bearwisdom::query::full_trace::trace_from_symbol(&db, sym, params.depth),
                None => bearwisdom::query::full_trace::trace_from_entry_points(&db, params.depth, params.max_traces),
            };
            match result {
                Ok(r) => ok_json(r).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

#[cfg(windows)]
fn list_windows_drives() -> Vec<String> {
    // Check A: through Z: by attempting to get metadata.
    (b'A'..=b'Z')
        .filter_map(|letter| {
            let drive = format!("{}:\\", letter as char);
            if std::path::Path::new(&drive).exists() {
                Some(drive)
            } else {
                None
            }
        })
        .collect()
}

// ===========================================================================
// MCP Audit log
// GET  /api/audit/sessions
// GET  /api/audit/calls?session_id=&limit=&offset=
// GET  /api/audit/stats
// GET  /api/audit/stream          (SSE)
// DELETE /api/audit/sessions/:id
// ===========================================================================

#[derive(Deserialize)]
pub struct AuditCallsQuery {
    path: String,
    session_id: String,
    #[serde(default = "default_audit_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_audit_limit() -> i64 { 100 }

pub async fn get_audit_sessions(
    State(state): State<AppState>,
    Query(params): Query<PathParam>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => match db.list_audit_sessions() {
            Ok(sessions) => ok_json(sessions).into_response(),
            Err(e) => err_json(e).into_response(),
        },
        Err(e) => err_json(e).into_response(),
    }
}

pub async fn get_audit_calls(
    State(state): State<AppState>,
    Query(params): Query<AuditCallsQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => {
            match db.list_audit_calls(&params.session_id, params.limit, params.offset) {
                Ok(calls) => ok_json(calls).into_response(),
                Err(e) => err_json(e).into_response(),
            }
        }
        Err(e) => err_json(e).into_response(),
    }
}

pub async fn get_audit_stats(
    State(state): State<AppState>,
    Query(params): Query<PathParam>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => match db.get_audit_stats() {
            Ok(stats) => ok_json(stats).into_response(),
            Err(e) => err_json(e).into_response(),
        },
        Err(e) => err_json(e).into_response(),
    }
}

/// SSE stream: emits new `AuditRecord[]` JSON arrays as they arrive, polling every 500 ms.
/// The client receives `[]` keep-alive payloads between real events.
pub async fn get_audit_stream(
    State(state): State<AppState>,
    Query(params): Query<PathParam>,
) -> axum::response::sse::Sse<
    impl futures::stream::Stream<
        Item = Result<axum::response::sse::Event, std::convert::Infallible>,
    >,
> {
    use axum::response::sse::{Event, KeepAlive};

    let path = PathBuf::from(&params.path);
    let pool = state.pool.clone();

    let stream = futures::stream::unfold(0i64, move |last_id| {
        let path = path.clone();
        let pool = pool.clone();
        async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            // rusqlite::Connection is !Send so it cannot cross an await; doing
            // all DB work inside the blocking closure keeps the async future Send.
            let (records, new_id) = tokio::task::spawn_blocking(move || {
                let db = match pool.get_db(&path) {
                    Ok(d) => d,
                    Err(_) => return (Vec::<bearwisdom::AuditRecord>::new(), last_id),
                };
                match db.list_new_audit_records(last_id) {
                    Ok(r) if !r.is_empty() => {
                        let new_id = r.last().map(|x| x.id).unwrap_or(last_id);
                        (r, new_id)
                    }
                    _ => (Vec::new(), last_id),
                }
            })
            .await
            .unwrap_or_else(|_| (Vec::new(), last_id));

            let data = if records.is_empty() {
                "[]".to_string()
            } else {
                serde_json::to_string(&records).unwrap_or_else(|_| "[]".to_string())
            };

            Some((Ok(Event::default().data(data)), new_id))
        }
    });

    axum::response::sse::Sse::new(stream).keep_alive(KeepAlive::default())
}

pub async fn delete_audit_session(
    State(state): State<AppState>,
    axum::extract::Path(session_id): axum::extract::Path<String>,
    Query(params): Query<PathParam>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match state.pool.get_db(&root) {
        Ok(db) => match db.delete_audit_session(&session_id) {
            Ok(deleted) => ok_json(json!({ "deleted": deleted })).into_response(),
            Err(e) => err_json(e).into_response(),
        },
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

fn default_level_packages() -> String { "packages".to_string() }

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
