use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

pub mod audit;
pub mod ops;
pub mod search;

pub use audit::{
    delete_audit_session, get_audit_calls, get_audit_sessions, get_audit_stats, get_audit_stream,
};
pub use ops::{
    get_architecture, get_dead_code, get_dependencies, get_embed_status, get_entry_points,
    get_flow_edges, get_full_trace, get_hierarchy, get_packages, get_status, get_trace_flow,
    get_workspace, post_embed, post_index,
};
pub use search::{
    get_blast_radius, get_browse, get_calls_in, get_calls_out, get_concept_members, get_concepts,
    get_definition, get_file_content, get_file_symbols, get_fuzzy_files, get_fuzzy_symbols,
    get_graph, get_grep, get_hybrid, get_references, get_search_content, get_search_symbols,
    get_symbol_info,
};

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

pub(crate) fn ok_json<T: serde::Serialize>(data: T) -> impl IntoResponse {
    Json(json!({"ok": true, "data": data}))
}

pub(crate) fn err_json(msg: impl std::fmt::Display) -> impl IntoResponse {
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
    pub(crate) path: String,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub(crate) path: String,
    pub(crate) q: Option<String>,
    #[serde(default = "default_limit_20")]
    pub(crate) limit: usize,
}

#[derive(Deserialize)]
pub struct SymbolQuery {
    pub(crate) path: String,
    pub(crate) symbol: String,
}

#[derive(Deserialize)]
pub struct SymbolLimitQuery {
    pub(crate) path: String,
    pub(crate) symbol: String,
    #[serde(default = "default_limit_100")]
    pub(crate) limit: usize,
}

#[derive(Deserialize)]
pub struct BlastQuery {
    pub(crate) path: String,
    pub(crate) symbol: String,
    #[serde(default = "default_depth_3")]
    pub(crate) depth: u32,
}

#[derive(Deserialize)]
pub struct FileQuery {
    pub(crate) path: String,
    pub(crate) file: String,
}

#[derive(Deserialize)]
pub struct GraphQuery {
    pub(crate) path: String,
    pub(crate) filter: Option<String>,
    #[serde(default = "default_max_nodes")]
    pub(crate) max_nodes: usize,
}

#[derive(Deserialize)]
pub struct ConceptMembersQuery {
    pub(crate) path: String,
    pub(crate) concept: String,
    #[serde(default = "default_limit_100")]
    pub(crate) limit: usize,
}

#[derive(Deserialize)]
pub struct GrepQuery {
    pub(crate) path: String,
    pub(crate) pattern: String,
    #[serde(default)]
    pub(crate) regex: bool,
    #[serde(default = "default_true")]
    pub(crate) case_insensitive: bool,
    #[serde(default = "default_limit_200")]
    pub(crate) limit: usize,
}

#[derive(Deserialize)]
pub struct BrowseQuery {
    pub(crate) path: Option<String>,
}

#[derive(Deserialize)]
pub struct DeadCodeQuery {
    pub(crate) path: String,
    pub(crate) scope: Option<String>,
    #[serde(default = "default_visibility_all")]
    pub(crate) visibility: String,
    #[serde(default)]
    pub(crate) include_tests: bool,
    #[serde(default = "default_limit_100")]
    pub(crate) limit: usize,
}

#[derive(Deserialize)]
pub struct IndexBody {
    pub(crate) path: String,
}

pub(crate) fn default_visibility_all() -> String { "all".to_string() }

pub(crate) fn default_limit_20() -> usize { 20 }
pub(crate) fn default_limit_100() -> usize { 100 }
pub(crate) fn default_limit_200() -> usize { 200 }
pub(crate) fn default_limit_500() -> usize { 500 }
pub(crate) fn default_depth_3() -> u32 { 3 }
pub(crate) fn default_max_nodes() -> usize { 500 }
pub(crate) fn default_true() -> bool { true }
pub(crate) fn default_forward() -> String { "forward".to_string() }
