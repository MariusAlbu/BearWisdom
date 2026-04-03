// =============================================================================
// connectors/connector_db.rs — Database operations for the connector layer
//
// All writes use INSERT OR IGNORE so the functions are idempotent and safe
// to call on re-index without manual cleanup first (except clear_connection_points,
// which does the explicit DELETE needed at the start of a full re-index).
// =============================================================================

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::types::{ConnectionPoint, ResolvedFlow};

// ---------------------------------------------------------------------------
// connection_points table
// ---------------------------------------------------------------------------

/// Delete all rows from `connection_points`.
///
/// Called at the beginning of a full re-index before connectors run,
/// so stale points from removed files are swept away.
pub fn clear_connection_points(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM connection_points", [])
        .context("Failed to clear connection_points")?;
    Ok(())
}

/// Bulk-insert a slice of `ConnectionPoint`s into the `connection_points` table.
///
/// Uses `INSERT OR IGNORE` so duplicate calls are safe.
pub fn store_connection_points(conn: &Connection, points: &[ConnectionPoint]) -> Result<()> {
    if points.is_empty() {
        return Ok(());
    }

    let mut stmt = conn
        .prepare_cached(
            "INSERT OR IGNORE INTO connection_points
                (file_id, symbol_id, line, protocol, direction, key, method, framework, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .context("Failed to prepare connection_points INSERT")?;

    for cp in points {
        stmt.execute(rusqlite::params![
            cp.file_id,
            cp.symbol_id,
            cp.line,
            cp.protocol.as_str(),
            cp.direction.as_str(),
            cp.key,
            cp.method,
            cp.framework,
            cp.metadata,
        ])
        .context("Failed to insert connection_point")?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// flow_edges table
// ---------------------------------------------------------------------------

/// Insert `ResolvedFlow`s into `flow_edges` and return the count written.
///
/// Uses `INSERT OR IGNORE` — duplicate (source_file_id, source_line,
/// source_symbol, target_file_id, edge_type) combos are silently skipped.
pub fn write_flow_edges(conn: &Connection, flows: &[ResolvedFlow]) -> Result<u32> {
    if flows.is_empty() {
        return Ok(0);
    }

    let mut stmt = conn
        .prepare_cached(
            "INSERT OR IGNORE INTO flow_edges
                (source_file_id, source_line, source_symbol, source_language,
                 target_file_id, target_line, target_symbol, target_language,
                 edge_type, protocol, http_method, url_pattern, confidence, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )
        .context("Failed to prepare flow_edges INSERT")?;

    let mut written = 0u32;

    for flow in flows {
        let start = &flow.start;
        let stop = &flow.stop;

        // http_method and url_pattern are REST-specific; for other protocols
        // we write empty strings / None so the columns stay queryable.
        let http_method = if start.method.is_empty() {
            None
        } else {
            Some(start.method.as_str())
        };
        let url_pattern = if start.key.is_empty() {
            None
        } else {
            Some(start.key.as_str())
        };

        // source_language / target_language: derive from the connection point
        // metadata if present, otherwise leave NULL — the column is nullable.
        let n = stmt
            .execute(rusqlite::params![
                start.file_id,
                start.line,
                // Use the key as a stand-in for source_symbol when no symbol_id.
                // Full symbol name resolution is left to enrichment passes.
                start.key,
                Option::<&str>::None, // source_language
                stop.file_id,
                stop.line,
                stop.key,
                Option::<&str>::None, // target_language
                flow.edge_type,
                start.protocol.as_str(),
                http_method,
                url_pattern,
                flow.confidence,
                start.metadata.as_deref(),
            ])
            .context("Failed to insert flow_edge")?;

        written += n as u32;
    }

    Ok(written)
}

// ---------------------------------------------------------------------------
// Backward compat: routes table
// ---------------------------------------------------------------------------

/// Populate the legacy `routes` table from REST stop connection_points.
///
/// Existing connectors and queries read from `routes`; this bridge keeps them
/// working during the migration period.  Only inserts rows that don't already
/// exist (`INSERT OR IGNORE`).
///
/// Returns the number of rows inserted.
pub fn populate_routes_from_stops(conn: &Connection) -> Result<u32> {
    let n = conn
        .execute(
            "INSERT OR IGNORE INTO routes
                (file_id, symbol_id, http_method, route_template, resolved_route, line)
             SELECT cp.file_id,
                    cp.symbol_id,
                    UPPER(cp.method),
                    cp.key,
                    cp.key,
                    cp.line
             FROM connection_points cp
             WHERE cp.protocol  = 'rest'
               AND cp.direction = 'stop'
               AND cp.method   != ''
               AND cp.key      != ''",
            [],
        )
        .context("Failed to populate routes from connection_points")?;

    Ok(n as u32)
}
