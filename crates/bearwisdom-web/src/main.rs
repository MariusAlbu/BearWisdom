// Force-link sqlite-vec native library into this binary.
extern crate sqlite_vec;

mod api;
mod db;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use clap::Parser;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

/// Shared embedding status visible to all request handlers.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EmbedStatus {
    pub state: &'static str, // "idle" | "running" | "done" | "error"
    pub embedded: u32,
    pub error: Option<String>,
}

/// Combined application state shared across all handlers.
#[derive(Clone)]
pub struct AppState {
    pub embed_status: Arc<Mutex<EmbedStatus>>,
    pub pool: Arc<db::PoolState>,
}

#[derive(Parser)]
#[command(name = "bw-web", about = "BearWisdom web UI server")]
struct Args {
    #[arg(long, default_value = "3030")]
    port: u16,
    #[arg(long)]
    static_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();

    let state = AppState {
        embed_status: Arc::new(Mutex::new(EmbedStatus {
            state: "idle",
            embedded: 0,
            error: None,
        })),
        pool: Arc::new(db::PoolState::new()),
    };

    let api_router = Router::new()
        .route("/index",          axum::routing::post(api::post_index))
        .route("/status",         axum::routing::get(api::get_status))
        .route("/architecture",   axum::routing::get(api::get_architecture))
        .route("/dead-code",      axum::routing::get(api::get_dead_code))
        .route("/entry-points",   axum::routing::get(api::get_entry_points))
        .route("/search-symbols", axum::routing::get(api::get_search_symbols))
        .route("/fuzzy-files",    axum::routing::get(api::get_fuzzy_files))
        .route("/fuzzy-symbols",  axum::routing::get(api::get_fuzzy_symbols))
        .route("/grep",           axum::routing::get(api::get_grep))
        .route("/search-content", axum::routing::get(api::get_search_content))
        .route("/hybrid",         axum::routing::get(api::get_hybrid))
        .route("/graph",          axum::routing::get(api::get_graph))
        .route("/concepts",       axum::routing::get(api::get_concepts))
        .route("/concept-members",axum::routing::get(api::get_concept_members))
        .route("/symbol-info",    axum::routing::get(api::get_symbol_info))
        .route("/definition",     axum::routing::get(api::get_definition))
        .route("/references",     axum::routing::get(api::get_references))
        .route("/calls-in",       axum::routing::get(api::get_calls_in))
        .route("/calls-out",      axum::routing::get(api::get_calls_out))
        .route("/blast-radius",   axum::routing::get(api::get_blast_radius))
        .route("/file-symbols",   axum::routing::get(api::get_file_symbols))
        .route("/file-content",   axum::routing::get(api::get_file_content))
        .route("/browse",         axum::routing::get(api::get_browse))
        .route("/embed",          axum::routing::post(api::post_embed))
        .route("/embed-status",   axum::routing::get(api::get_embed_status))
        .route("/flow-edges",     axum::routing::get(api::get_flow_edges))
        .route("/trace-flow",     axum::routing::get(api::get_trace_flow))
        .route("/full-trace",              axum::routing::get(api::get_full_trace))
        .route("/audit/sessions",          axum::routing::get(api::get_audit_sessions))
        .route("/audit/calls",             axum::routing::get(api::get_audit_calls))
        .route("/audit/stats",             axum::routing::get(api::get_audit_stats))
        .route("/audit/stream",            axum::routing::get(api::get_audit_stream))
        .route("/audit/sessions/{id}",      axum::routing::delete(api::delete_audit_session))
        .route("/hierarchy",               axum::routing::get(api::get_hierarchy))
        .route("/packages",                axum::routing::get(api::get_packages))
        .route("/workspace",               axum::routing::get(api::get_workspace))
        .route("/dependencies",            axum::routing::get(api::get_dependencies))
        .with_state(state);

    let mut app = Router::new()
        .nest("/api", api_router)
        .layer(CorsLayer::permissive());

    if let Some(dir) = args.static_dir {
        app = app.fallback_service(ServeDir::new(dir));
    }

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], args.port));
    tracing::info!("Listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
