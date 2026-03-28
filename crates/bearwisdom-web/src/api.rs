use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::db::{db_exists, open_existing_db, resolve_db_path};
use crate::SharedEmbedStatus;

// ---------------------------------------------------------------------------
// Directory-based concept discovery (fallback for languages without dotted namespaces)
// ---------------------------------------------------------------------------

/// Groups symbols by the first 2 directory segments of their file path.
/// E.g. `crates/bearwisdom/src/query/blast_radius.rs` → concept `"crates/bearwisdom"`.
fn discover_directory_concepts(db: &bearwisdom::Database) -> anyhow::Result<()> {
    let conn = &db.conn;

    // Find distinct directory prefixes (first 2 segments).
    let prefixes: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT
                 CASE
                   WHEN instr(substr(path, instr(path, '/') + 1), '/') > 0
                   THEN substr(path, 1,
                        instr(path, '/') +
                        instr(substr(path, instr(path, '/') + 1), '/') - 1)
                   ELSE
                     CASE WHEN instr(path, '/') > 0
                     THEN substr(path, 1, instr(path, '/') - 1)
                     ELSE NULL END
                 END AS dir_prefix
             FROM files
             WHERE dir_prefix IS NOT NULL"
        )?;

        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.filter_map(|r| r.ok())
            .filter(|p| !p.is_empty())
            .collect()
    };

    for prefix in &prefixes {
        // Count symbols in this directory prefix to skip tiny groups.
        let count: u32 = conn.query_row(
            "SELECT COUNT(*) FROM symbols s JOIN files f ON s.file_id = f.id
             WHERE f.path LIKE ?1 || '/%'",
            rusqlite::params![prefix],
            |r| r.get(0),
        ).unwrap_or(0);

        if count < 3 { continue; } // Skip trivially small groups

        let auto_pattern = format!("{prefix}/*");
        conn.execute(
            "INSERT OR IGNORE INTO concepts (name, auto_pattern, created_at)
             VALUES (?1, ?2, strftime('%s', 'now'))",
            rusqlite::params![prefix, auto_pattern],
        )?;

        // Assign members: all symbols whose file path starts with this prefix.
        conn.execute(
            "INSERT OR IGNORE INTO concept_members (concept_id, symbol_id, auto_assigned)
             SELECT c.id, s.id, 1
             FROM concepts c, symbols s
             JOIN files f ON s.file_id = f.id
             WHERE c.name = ?1
               AND f.path LIKE ?1 || '/%'",
            rusqlite::params![prefix],
        )?;
    }

    Ok(())
}

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

pub async fn post_index(Json(body): Json<IndexBody>) -> impl IntoResponse {
    let root = PathBuf::from(&body.path);
    match index_project(&root) {
        Ok(stats) => ok_json(stats).into_response(),
        Err(e) => err_json(e).into_response(),
    }
}

fn index_project(root: &Path) -> anyhow::Result<serde_json::Value> {
    let db_path = resolve_db_path(root)?;
    let already_existed = db_exists(root);

    let mut db = bearwisdom::Database::open_with_vec(&db_path)?;

    if already_existed {
        // DB exists — read stats without re-indexing
        let conn = &db.conn;
        let file_count: u32 = conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        let symbol_count: u32 = conn.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))?;
        let edge_count: u32 = conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
        let unresolved: u32 = conn.query_row("SELECT COUNT(*) FROM unresolved_refs", [], |r| r.get(0)).unwrap_or(0);

        return Ok(json!({
            "file_count": file_count,
            "symbol_count": symbol_count,
            "edge_count": edge_count,
            "unresolved_ref_count": unresolved,
            "duration_ms": 0,
            "cached": true,
        }));
    }

    let stats = bearwisdom::full_index(&mut db, root, None, None)?;
    let _ = bearwisdom::query::concepts::discover_concepts(&db);
    let _ = bearwisdom::query::concepts::auto_assign_concepts(&db);

    // If no concepts were discovered (flat qualified names), create directory-based concepts.
    let concept_count: u32 = db.conn.query_row(
        "SELECT COUNT(*) FROM concepts", [], |r| r.get(0)
    ).unwrap_or(0);
    if concept_count == 0 {
        let _ = discover_directory_concepts(&db);
    }

    // Embedding runs separately — too slow to block the index response.
    // Use `bw embed` CLI or the hybrid search will embed on first query.

    Ok(json!({
        "file_count": stats.file_count,
        "symbol_count": stats.symbol_count,
        "edge_count": stats.edge_count,
        "unresolved_ref_count": stats.unresolved_ref_count,
        "duration_ms": stats.duration_ms,
        "cached": false,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/status
// ---------------------------------------------------------------------------

pub async fn get_status(Query(params): Query<PathParam>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match status_counts(&root) {
        Ok(v) => ok_json(v).into_response(),
        Err(e) => err_json(e).into_response(),
    }
}

fn status_counts(root: &Path) -> anyhow::Result<serde_json::Value> {
    let db = open_existing_db(root)?;
    let conn = &db.conn;
    let file_count: i64 = conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
    let symbol_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))?;
    let edge_count: i64 = conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
    Ok(json!({
        "file_count": file_count,
        "symbol_count": symbol_count,
        "edge_count": edge_count,
    }))
}

// ---------------------------------------------------------------------------
// GET /api/architecture
// ---------------------------------------------------------------------------

pub async fn get_architecture(Query(params): Query<PathParam>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match open_existing_db(&root) {
        Ok(db) => match bearwisdom::query::architecture::get_overview(&db) {
            Ok(overview) => ok_json(overview).into_response(),
            Err(e) => err_json(e).into_response(),
        },
        Err(e) => err_json(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /api/search-symbols
// ---------------------------------------------------------------------------

pub async fn get_search_symbols(Query(params): Query<SearchQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    let query = params.q.unwrap_or_default();
    match open_existing_db(&root) {
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

pub async fn get_fuzzy_files(Query(params): Query<SearchQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    let query = params.q.unwrap_or_default();
    match open_existing_db(&root) {
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

pub async fn get_fuzzy_symbols(Query(params): Query<SearchQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    let query = params.q.unwrap_or_default();
    match open_existing_db(&root) {
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

pub async fn get_search_content(Query(params): Query<SearchQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match open_existing_db(&root) {
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

pub async fn get_hybrid(Query(params): Query<SearchQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match open_existing_db(&root) {
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

pub async fn get_graph(Query(params): Query<GraphQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    let filter = params.filter.as_deref();
    match open_existing_db(&root) {
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

pub async fn get_concepts(Query(params): Query<PathParam>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match open_existing_db(&root) {
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
    Query(params): Query<ConceptMembersQuery>,
) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match open_existing_db(&root) {
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

pub async fn get_symbol_info(Query(params): Query<SymbolQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match open_existing_db(&root) {
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

pub async fn get_definition(Query(params): Query<SymbolQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match open_existing_db(&root) {
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

pub async fn get_references(Query(params): Query<SymbolLimitQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match open_existing_db(&root) {
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

pub async fn get_calls_in(Query(params): Query<SymbolLimitQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match open_existing_db(&root) {
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

pub async fn get_calls_out(Query(params): Query<SymbolLimitQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match open_existing_db(&root) {
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

pub async fn get_blast_radius(Query(params): Query<BlastQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match open_existing_db(&root) {
        Ok(db) => {
            match bearwisdom::query::blast_radius::blast_radius(
                &db,
                &params.symbol,
                params.depth,
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

pub async fn get_file_symbols(Query(params): Query<FileQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match file_symbols(&root, &params.file) {
        Ok(v) => ok_json(v).into_response(),
        Err(e) => err_json(e).into_response(),
    }
}

#[derive(serde::Serialize)]
struct FileSymbolRow {
    name: String,
    qualified_name: String,
    kind: String,
    line: u32,
    col: u32,
    end_line: u32,
    scope_path: Option<String>,
    signature: Option<String>,
    visibility: Option<String>,
}

fn file_symbols(root: &Path, file: &str) -> anyhow::Result<Vec<FileSymbolRow>> {
    let db = open_existing_db(root)?;
    let conn = &db.conn;
    let mut stmt = conn.prepare(
        "SELECT s.name, s.qualified_name, s.kind, s.line, s.col, s.end_line,
                s.scope_path, s.signature, s.visibility
         FROM symbols s
         JOIN files f ON s.file_id = f.id
         WHERE f.path = ?1
         ORDER BY s.line",
    )?;
    let rows = stmt.query_map([file], |row| {
        Ok(FileSymbolRow {
            name:           row.get(0)?,
            qualified_name: row.get(1)?,
            kind:           row.get(2)?,
            line:           row.get(3)?,
            col:            row.get(4)?,
            end_line:       row.get(5)?,
            scope_path:     row.get(6)?,
            signature:      row.get(7)?,
            visibility:     row.get(8)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(anyhow::Error::from)
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
    State(status): State<SharedEmbedStatus>,
    Json(body): Json<IndexBody>,
) -> impl IntoResponse {
    // Reject if already running.
    {
        let s = status.lock().unwrap();
        if s.state == "running" {
            return ok_json(json!({"started": false, "reason": "already running"})).into_response();
        }
    }

    // Mark as running immediately.
    {
        let mut s = status.lock().unwrap();
        s.state = "running";
        s.embedded = 0;
        s.error = None;
    }

    let root = PathBuf::from(&body.path);
    let bg_status = status.clone();

    // Run embedding in a background thread — don't block the request.
    tokio::task::spawn_blocking(move || {
        let result = (|| -> anyhow::Result<u32> {
            let db_path = resolve_db_path(&root)?;
            let db = bearwisdom::Database::open_with_vec(&db_path)?;
            let model_dir = bearwisdom::search::embedder::Embedder::resolve_model_dir(&root)
                .ok_or_else(|| anyhow::anyhow!("No CodeRankEmbed model found"))?;
            let mut embedder = bearwisdom::search::embedder::Embedder::new(model_dir);
            let (n, _) = bearwisdom::embed_chunks(&db.conn, &mut embedder, 4)?;
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

pub async fn get_embed_status(State(status): State<SharedEmbedStatus>) -> impl IntoResponse {
    let s = status.lock().unwrap();
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

#[derive(serde::Serialize)]
struct FlowEdgeRow {
    source_file: String,
    source_line: Option<i64>,
    source_symbol: Option<String>,
    source_language: String,
    target_file: Option<String>,
    target_line: Option<i64>,
    target_symbol: Option<String>,
    target_language: String,
    edge_type: String,
    protocol: Option<String>,
    url_pattern: Option<String>,
}

pub async fn get_flow_edges(Query(params): Query<FlowEdgesQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match open_existing_db(&root) {
        Ok(db) => match query_flow_edges(&db, params.limit) {
            Ok(v) => ok_json(v).into_response(),
            Err(e) => err_json(e).into_response(),
        },
        Err(e) => err_json(e).into_response(),
    }
}

fn query_flow_edges(db: &bearwisdom::Database, limit: usize) -> anyhow::Result<serde_json::Value> {
    use std::collections::HashMap;

    let conn = &db.conn;

    // Summary counts from the full dataset (before limit).
    let mut by_edge_type: HashMap<String, u32> = HashMap::new();
    let mut by_language_pair: HashMap<String, u32> = HashMap::new();
    let total: u32 = {
        let mut stmt = conn.prepare(
            "SELECT fe.edge_type,
                    COALESCE(fe.source_language, sf.language, '') AS src_lang,
                    COALESCE(fe.target_language, tf.language, '') AS tgt_lang,
                    COUNT(*) AS cnt
             FROM flow_edges fe
             JOIN files sf ON sf.id = fe.source_file_id
             LEFT JOIN files tf ON tf.id = fe.target_file_id
             GROUP BY fe.edge_type, src_lang, tgt_lang"
        )?;
        let mut total = 0u32;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let et: String = row.get(0)?;
            let src: String = row.get::<_, Option<String>>(1)?.unwrap_or_default();
            let tgt: String = row.get::<_, Option<String>>(2)?.unwrap_or_default();
            let cnt: u32 = row.get(3)?;
            *by_edge_type.entry(et).or_default() += cnt;
            let pair = format!("{src} → {tgt}");
            *by_language_pair.entry(pair).or_default() += cnt;
            total += cnt;
        }
        total
    };

    // Interleave edge types so the limit gets a fair mix.
    let mut stmt = conn.prepare(
        "SELECT source_file, source_line, source_symbol, source_language,
                target_file, target_line, target_symbol, target_language,
                edge_type, protocol, url_pattern
         FROM (
             SELECT
                 sf.path                                       AS source_file,
                 fe.source_line,
                 fe.source_symbol,
                 COALESCE(fe.source_language, sf.language, '') AS source_language,
                 tf.path                                       AS target_file,
                 fe.target_line,
                 fe.target_symbol,
                 COALESCE(fe.target_language, tf.language, '') AS target_language,
                 fe.edge_type,
                 fe.protocol,
                 fe.url_pattern,
                 ROW_NUMBER() OVER (PARTITION BY fe.edge_type ORDER BY sf.path, fe.source_line) AS rn
             FROM flow_edges fe
             JOIN files sf ON sf.id = fe.source_file_id
             LEFT JOIN files tf ON tf.id = fe.target_file_id
         )
         ORDER BY rn, edge_type
         LIMIT ?1",
    )?;

    let rows = stmt.query_map([limit as i64], |row| {
        Ok(FlowEdgeRow {
            source_file:     row.get(0)?,
            source_line:     row.get(1)?,
            source_symbol:   row.get(2)?,
            source_language: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            target_file:     row.get(4)?,
            target_line:     row.get(5)?,
            target_symbol:   row.get(6)?,
            target_language: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
            edge_type:       row.get(8)?,
            protocol:        row.get(9)?,
            url_pattern:     row.get(10)?,
        })
    })?
    .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(json!({
        "edges": rows,
        "summary": {
            "total": total,
            "by_edge_type": by_edge_type,
            "by_language_pair": by_language_pair,
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

pub async fn get_trace_flow(Query(params): Query<TraceFlowQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match open_existing_db(&root) {
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

pub async fn get_full_trace(Query(params): Query<FullTraceQuery>) -> impl IntoResponse {
    let root = PathBuf::from(&params.path);
    match open_existing_db(&root) {
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
