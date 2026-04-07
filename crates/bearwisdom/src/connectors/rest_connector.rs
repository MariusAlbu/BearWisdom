// =============================================================================
// connectors/rest_connector.rs — REST/HTTP connector (new architecture)
//
// Wraps the existing route detection logic (http_api, frontend_http,
// dotnet_http_client, spring, fastapi, go_routes, rails, laravel, nestjs)
// and emits ConnectionPoints instead of writing flow_edges directly.
//
// Stop points: backend route handlers (from the `routes` table).
// Start points: HTTP client calls (fetch, axios, HttpClient, requests, etc.)
// =============================================================================

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::traits::{Connector, ConnectorDescriptor};
use super::types::{ConnectionPoint, FlowDirection, Protocol};
use crate::indexer::project_context::ProjectContext;

pub struct RestConnector;

impl Connector for RestConnector {
    fn descriptor(&self) -> ConnectorDescriptor {
        ConnectorDescriptor {
            name: "rest",
            protocols: &[Protocol::Rest],
            languages: &[
                "csharp", "typescript", "tsx", "javascript", "jsx", "python",
                "go", "java", "kotlin", "ruby", "php", "rust", "dart", "swift",
            ],
        }
    }

    fn detect(&self, _ctx: &ProjectContext) -> bool {
        // REST is so ubiquitous that it's always worth running.
        true
    }

    fn extract(
        &self,
        conn: &Connection,
        project_root: &Path,
    ) -> Result<Vec<ConnectionPoint>> {
        let mut points = Vec::new();

        // --- Stop points: backend route handlers from the `routes` table ---
        extract_route_stops(conn, &mut points)?;

        // --- Start points: frontend HTTP calls (all languages) ---
        extract_http_call_starts(conn, project_root, &mut points)?;

        Ok(points)
    }
}

// ---------------------------------------------------------------------------
// Stop extraction — from the `routes` table
// ---------------------------------------------------------------------------

fn extract_route_stops(
    conn: &Connection,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT r.file_id, r.symbol_id, r.line, r.http_method,
                    COALESCE(r.resolved_route, r.route_template)
             FROM routes r
             WHERE r.http_method != '' AND r.route_template != ''",
        )
        .context("Failed to prepare routes query for REST stops")?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<u32>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .context("Failed to query routes")?;

    for row in rows {
        let (file_id, symbol_id, line, method, route) =
            row.context("Failed to read route row")?;

        out.push(ConnectionPoint {
            file_id,
            symbol_id,
            line: line.unwrap_or(0),
            protocol: Protocol::Rest,
            direction: FlowDirection::Stop,
            key: route,
            method: method.to_uppercase(),
            framework: String::new(),
            metadata: None,
        });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Start extraction — HTTP client calls (delegates to frontend_http)
// ---------------------------------------------------------------------------

fn extract_http_call_starts(
    conn: &Connection,
    project_root: &Path,
    out: &mut Vec<ConnectionPoint>,
) -> Result<()> {
    // TS/JS/Python/Go/Java/Ruby/C# regex-based detection.
    let calls =
        super::frontend_http::detect_http_calls_all_languages(conn, project_root)
            .context("REST start detection (frontend_http) failed")?;

    for call in calls {
        out.push(ConnectionPoint {
            file_id: call.file_id,
            symbol_id: call.symbol_id,
            line: call.line,
            protocol: Protocol::Rest,
            direction: FlowDirection::Start,
            key: call.url_pattern,
            method: call.http_method,
            framework: String::new(),
            metadata: None,
        });
    }

    // .NET HttpClient pattern detection (GetAsync, PostAsync, UriHelper, etc.)
    let dotnet_calls =
        super::dotnet_http_client::detect_dotnet_http_calls(conn, project_root)
            .context("REST start detection (dotnet_http_client) failed")?;

    for call in dotnet_calls {
        out.push(ConnectionPoint {
            file_id: call.file_id,
            symbol_id: None,
            line: call.line,
            protocol: Protocol::Rest,
            direction: FlowDirection::Start,
            key: call.url_pattern,
            method: call.http_method,
            framework: "dotnet_http".to_string(),
            metadata: None,
        });
    }

    Ok(())
}
