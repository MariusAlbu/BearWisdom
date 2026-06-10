use std::path::PathBuf;

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use crate::api::{err_json, ok_json, PathParam};
use crate::AppState;

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

fn default_audit_limit() -> i64 {
    100
}

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
        Ok(db) => match db.list_audit_calls(&params.session_id, params.limit, params.offset) {
            Ok(calls) => ok_json(calls).into_response(),
            Err(e) => err_json(e).into_response(),
        },
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
    impl futures::stream::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
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
