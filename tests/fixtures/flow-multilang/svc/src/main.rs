use axum::{Router, routing::get};

pub fn make_app() -> Router<()> {
    Router::new()
        .route("/api/items/:id", get(get_item))
        .route("/api/health", get(health))
}

async fn get_item() -> &'static str { "item" }
async fn health() -> &'static str { "ok" }
