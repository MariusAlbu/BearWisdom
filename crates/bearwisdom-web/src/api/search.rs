use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use serde_json::json;

use crate::api::{
    err_json, ok_json, BlastQuery, BrowseQuery, ConceptMembersQuery, FileQuery, GraphQuery,
    GrepQuery, PathParam, SearchQuery, SymbolLimitQuery, SymbolQuery,
};
use crate::AppState;

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
            match bearwisdom::query::search::search_symbols(
                &db,
                &query,
                params.limit,
                &bearwisdom::query::QueryOptions::full(),
            ) {
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
        Ok(db) => match bearwisdom::search::fuzzy::FuzzyIndex::from_db(&db) {
            Ok(idx) => ok_json(idx.match_files(&query, params.limit)).into_response(),
            Err(e) => err_json(e).into_response(),
        },
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
        Ok(db) => match bearwisdom::search::fuzzy::FuzzyIndex::from_db(&db) {
            Ok(idx) => ok_json(idx.match_symbols(&query, params.limit)).into_response(),
            Err(e) => err_json(e).into_response(),
        },
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
            match bearwisdom::search::content_search::search_content(
                &db,
                query,
                &scope,
                params.limit,
            ) {
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
            match bearwisdom::search::hybrid::hybrid_search(
                &db,
                &mut embedder,
                query,
                &scope,
                params.limit,
            ) {
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
        Ok(db) => match bearwisdom::query::subgraph::export_graph(&db, filter, params.max_nodes) {
            Ok(result) => ok_json(result).into_response(),
            Err(e) => err_json(e).into_response(),
        },
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
            match bearwisdom::query::concepts::concept_members(&db, &params.concept, params.limit) {
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
        Ok(db) => match bearwisdom::query::symbol_info::symbol_info(
            &db,
            &params.symbol,
            &bearwisdom::query::QueryOptions::full(),
        ) {
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
        Ok(db) => match bearwisdom::query::definitions::goto_definition(&db, &params.symbol) {
            Ok(defs) => ok_json(defs).into_response(),
            Err(e) => err_json(e).into_response(),
        },
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
            match bearwisdom::query::references::find_references(&db, &params.symbol, params.limit)
            {
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
    Ok(bearwisdom::query::symbol_info::file_symbols(
        &db,
        file,
        bearwisdom::FileSymbolsMode::Full,
    )?)
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
