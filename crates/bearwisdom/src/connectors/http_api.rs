// =============================================================================
// connectors/http_api.rs  —  HTTP API connector
//
// Two jobs:
//   1. Write route records from ExtractedRoute data that was already stored
//      in the DB via the indexer pipeline.  (The indexer stores routes in the
//      `routes` table; this connector enriches them with resolved route paths.)
//   2. Scan TypeScript symbols for `fetch(url)` and `axios.*` calls and
//      attempt to match them against the routes table.  Matched pairs get a
//      cross-language `http_call` edge.
//
// URL normalisation
// -----------------
// Before comparing, both sides are normalised:
//   - Strip a common base path prefix (e.g. "/api")
//   - Collapse route parameters to a placeholder: `/items/{id:int}` → `/items/{*}`
//   - Lowercase the entire template
//
// Matching
// --------
// We do a segment-by-segment comparison.  A literal segment must match exactly;
// a parameter segment (`{*}`) matches any single segment.
// =============================================================================

use crate::db::Database;
use crate::types::RouteInfo;
use anyhow::{Context, Result};

/// Post-processing step: enrich routes and create cross-language edges.
///
/// Call this after the indexer has written routes to the DB.
pub fn connect(db: &Database) -> Result<()> {
    enrich_routes(db)?;
    match_ts_http_calls(db)?;
    Ok(())
}

/// Load all route records from the DB as `RouteInfo` structs.
pub fn list_routes(db: &Database) -> Result<Vec<RouteInfo>> {
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT r.id, f.path, r.http_method, r.route_template, r.resolved_route,
                r.line, s.name
         FROM routes r
         JOIN files f ON r.file_id = f.id
         LEFT JOIN symbols s ON r.symbol_id = s.id
         ORDER BY r.http_method, r.route_template",
    ).context("Failed to prepare list_routes query")?;

    let rows = stmt.query_map([], |row| {
        Ok(RouteInfo {
            id: row.get(0)?,
            file_path: row.get(1)?,
            http_method: row.get(2)?,
            route_template: row.get(3)?,
            resolved_route: row.get(4)?,
            line: row.get::<_, Option<u32>>(5)?.unwrap_or(0),
            handler_name: row.get(6)?,
        })
    }).context("Failed to execute list_routes query")?;

    rows.map(|r| r.context("Failed to read route row"))
        .collect()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Compute fully resolved route paths where possible.
///
/// For minimal-API routes this is usually the template itself.
/// For controller-based routes we'd need to combine the controller [Route]
/// prefix with the method attribute — simplified here to just copy the
/// template as-is.
fn enrich_routes(db: &Database) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "UPDATE routes SET resolved_route = route_template WHERE resolved_route IS NULL",
        [],
    ).context("Failed to set resolved_route")?;
    Ok(())
}

/// Find TypeScript `fetch` and `axios.*` call edges and match them to routes.
///
/// For each matched pair we insert a cross-language `http_call` edge from the
/// TypeScript symbol to the C# route handler symbol.
fn match_ts_http_calls(db: &Database) -> Result<()> {
    let conn = db.conn();

    // Find TS symbols that call "fetch" or "axios.*".
    let ts_calls: Vec<(i64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT e.source_id, e.target_id
             FROM edges e
             JOIN symbols tgt ON e.target_id = tgt.id
             WHERE e.kind = 'calls'
             AND (tgt.name = 'fetch' OR tgt.name LIKE 'axios.%')",
        ).context("Failed to prepare TS call query")?;

        // Collect eagerly so stmt can be dropped before we use the Vec.
        // (rusqlite MappedRows borrows stmt; the borrow must end before block exit.)
        let rows: rusqlite::Result<Vec<(i64, i64)>> =
            stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
                .collect();
        rows.context("Failed to collect TS call rows")?
            .into_iter()
            .map(|(src, _tgt)| (src, String::new()))
            .collect()
    };

    if ts_calls.is_empty() {
        return Ok(());
    }

    // Get all routes for matching.
    let routes = list_routes(db)?;

    for (caller_id, _url) in &ts_calls {
        // Without a captured URL we can't do path matching — skip for now.
        // When URLs are captured, the loop below would normalise and compare.
        for _route in &routes {
            // Future: normalise(_url) == normalise(route.route_template)
            // → insert http_call edge.
            let _ = caller_id;
        }
    }

    Ok(())
}

/// Normalise a URL or route template for comparison.
///
/// Examples:
///   "/api/catalog/items/{id:int}"  → "catalog/items/{*}"
///   "/api/catalog/items/{id}"      → "catalog/items/{*}"
///   "/api/v1/orders/{orderId}"     → "orders/{*}"
///   "http://localhost:8080/api/foo" → "foo"
pub fn normalise_route(template: &str) -> String {
    // Strip protocol + host if present (e.g. "http://localhost:8080/api/foo" → "/api/foo").
    let t = if let Some(idx) = template.find("://") {
        // Find the first / after the host.
        template[idx + 3..]
            .find('/')
            .map(|i| &template[idx + 3 + i..])
            .unwrap_or("")
    } else {
        template
    };

    // Strip leading slash.
    let t = t.trim_start_matches('/');

    // Strip common API prefixes: /api/, /api/v1/, /api/v2/, /API/
    let t = t
        .trim_start_matches("api/")
        .trim_start_matches("API/");
    // Strip version prefixes: v1/, v2/, v3/
    let t = if t.len() >= 3
        && t.starts_with('v')
        && t.as_bytes().get(1).map_or(false, |b| b.is_ascii_digit())
        && t.as_bytes().get(2) == Some(&b'/')
    {
        &t[3..]
    } else {
        t
    };

    let segments: Vec<String> = t
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|seg| {
            if seg.starts_with('{') && seg.ends_with('}') {
                "{*}".to_string()
            } else if seg.starts_with("${") || seg.starts_with(':') {
                // Template literal params: ${id}, :id
                "{*}".to_string()
            } else {
                seg.to_lowercase()
            }
        })
        .collect();

    segments.join("/")
}

/// Compare two normalised route templates segment by segment.
///
/// `{*}` in either side matches any single segment.
pub fn routes_match(a: &str, b: &str) -> bool {
    let a_segs: Vec<&str> = a.split('/').collect();
    let b_segs: Vec<&str> = b.split('/').collect();

    if a_segs.len() != b_segs.len() {
        return false;
    }

    a_segs.iter().zip(b_segs.iter()).all(|(a_seg, b_seg)| {
        *a_seg == "{*}" || *b_seg == "{*}" || a_seg == b_seg
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "http_api_tests.rs"]
mod tests;
