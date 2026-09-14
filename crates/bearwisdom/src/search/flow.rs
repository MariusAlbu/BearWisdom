// =============================================================================
// search/flow.rs  —  Cross-language flow graph traversal  (Phase 5)
//
// Traverses the `flow_edges` table using recursive CTEs to trace execution
// paths that cross language boundaries (TypeScript → C# → SQL, etc.).
// =============================================================================

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::db::Database;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// One hop in a cross-language flow trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowStep {
    /// Stable database identity of the flow edge represented by this hop.
    pub edge_id: i64,
    /// Edge that led to this hop. `None` marks a root edge from the requested
    /// source/target location.
    pub parent_edge_id: Option<i64>,
    /// Distance from the requested location (0 = directly incident edge).
    pub depth: u32,
    /// Source endpoint file.
    pub file_path: String,
    /// Source endpoint line, if known.
    pub line: Option<u32>,
    /// Source endpoint symbol, if known.
    pub symbol: Option<String>,
    /// Source endpoint language.
    pub language: String,
    /// Target endpoint file. Missing for a single-ended observation.
    pub target_file_path: Option<String>,
    /// Target endpoint line, if resolved.
    pub target_line: Option<u32>,
    /// Target endpoint symbol, if resolved.
    pub target_symbol: Option<String>,
    /// Target endpoint language, if resolved.
    pub target_language: Option<String>,
    /// Whether the observation has a concrete target file.
    pub paired: bool,
    /// Semantic kind of the edge (e.g. `http_call`, `rpc_call`).
    pub edge_type: String,
    /// Transport protocol if applicable.
    pub protocol: Option<String>,
    /// HTTP verb if applicable.
    pub http_method: Option<String>,
    /// Normalized route/channel key used to pair the endpoints.
    pub url_pattern: Option<String>,
    /// Resolver/pairer confidence retained as provenance.
    pub confidence: f64,
}

fn flow_step_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FlowStep> {
    let target_file_path = row.get::<_, Option<String>>(7)?;
    Ok(FlowStep {
        depth: row.get(0)?,
        edge_id: row.get(1)?,
        parent_edge_id: row.get(2)?,
        file_path: row.get(3)?,
        line: row.get(4)?,
        symbol: row.get(5)?,
        language: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        target_file_path: target_file_path.clone(),
        target_line: row.get(8)?,
        target_symbol: row.get(9)?,
        target_language: row.get(10)?,
        paired: target_file_path.is_some(),
        edge_type: row.get(11)?,
        protocol: row.get(12)?,
        http_method: row.get(13)?,
        url_pattern: row.get(14)?,
        confidence: row.get(15)?,
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Trace the flow graph starting from a file + line, up to `max_depth` hops.
///
/// Uses a recursive CTE on the `flow_edges` table.  The result is ordered by
/// depth then file path, so callers can render a deterministic timeline.
pub fn trace_flow(
    db: &Database,
    start_file: &str,
    start_line: u32,
    max_depth: u32,
) -> Result<Vec<FlowStep>> {
    let conn = db.conn();

    // Each row is an edge, with both endpoints retained. Recursion advances
    // only when the preceding target has an exact source location (preferred)
    // or symbol identity. A file match alone is never enough: it would splice
    // unrelated routes, jobs, or database operations from the same file into
    // one apparent path.
    let sql = "
        WITH RECURSIVE flow_trace(
            depth, edge_id, parent_edge_id,
            source_file_id, source_line, source_symbol, source_language,
            target_file_id, target_line, target_symbol, target_language,
            edge_type, protocol, http_method, url_pattern, confidence,
            edge_path
        ) AS (
            SELECT
                0,
                fe.id,
                NULL,
                fe.source_file_id,
                fe.source_line,
                fe.source_symbol,
                fe.source_language,
                fe.target_file_id,
                fe.target_line,
                fe.target_symbol,
                fe.target_language,
                fe.edge_type,
                fe.protocol,
                fe.http_method,
                fe.url_pattern,
                fe.confidence,
                printf(',%d,', fe.id)
            FROM flow_edges fe
            JOIN files f ON f.id = fe.source_file_id
            WHERE f.path = ?1
              AND (?2 = 0 OR fe.source_line = ?2)

            UNION ALL

            SELECT
                ft.depth + 1,
                fe.id,
                ft.edge_id,
                fe.source_file_id,
                fe.source_line,
                fe.source_symbol,
                fe.source_language,
                fe.target_file_id,
                fe.target_line,
                fe.target_symbol,
                fe.target_language,
                fe.edge_type,
                fe.protocol,
                fe.http_method,
                fe.url_pattern,
                fe.confidence,
                ft.edge_path || fe.id || ','
            FROM flow_trace ft
            JOIN flow_edges fe
              ON fe.source_file_id = ft.target_file_id
             AND (
                    (ft.target_line IS NOT NULL AND fe.source_line = ft.target_line)
                 OR (ft.target_line IS NULL AND ft.target_symbol IS NOT NULL
                     AND fe.source_symbol = ft.target_symbol)
            )
            WHERE ft.depth < ?3
              AND instr(ft.edge_path, printf(',%d,', fe.id)) = 0
        )
        SELECT DISTINCT
            ft.depth,
            ft.edge_id,
            ft.parent_edge_id,
            sf.path,
            ft.source_line,
            ft.source_symbol,
            COALESCE(ft.source_language, sf.language),
            tf.path,
            ft.target_line,
            ft.target_symbol,
            COALESCE(ft.target_language, tf.language),
            ft.edge_type,
            ft.protocol,
            ft.http_method,
            ft.url_pattern,
            ft.confidence
        FROM flow_trace ft
        JOIN files sf ON sf.id = ft.source_file_id
        LEFT JOIN files tf ON tf.id = ft.target_file_id
        ORDER BY ft.depth, ft.edge_id
    ";

    let mut stmt = conn
        .prepare(sql)
        .context("Failed to prepare trace_flow CTE")?;

    let steps = stmt
        .query_map(
            rusqlite::params![start_file, start_line, max_depth],
            flow_step_from_row,
        )
        .context("Failed to execute trace_flow CTE")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect trace_flow results")?;

    tracing::debug!(
        start_file,
        start_line,
        max_depth,
        steps = steps.len(),
        "trace_flow complete"
    );

    Ok(steps)
}

/// Trace the flow graph BACKWARD from a file + line, up to `max_depth` hops.
///
/// Where `trace_flow` follows edges forward (source → target), this function
/// follows them backward (target → source), answering "what flows INTO this
/// node?"  The depth counter still increments per hop so callers can interpret
/// it as distance from the start node in the reverse direction.
pub fn trace_flow_reverse(
    db: &Database,
    start_file: &str,
    start_line: u32,
    max_depth: u32,
) -> Result<Vec<FlowStep>> {
    let conn = db.conn();

    let sql = "
        WITH RECURSIVE flow_trace(
            depth, edge_id, parent_edge_id,
            source_file_id, source_line, source_symbol, source_language,
            target_file_id, target_line, target_symbol, target_language,
            edge_type, protocol, http_method, url_pattern, confidence,
            edge_path
        ) AS (
            SELECT
                0,
                fe.id,
                NULL,
                fe.source_file_id,
                fe.source_line,
                fe.source_symbol,
                fe.source_language,
                fe.target_file_id,
                fe.target_line,
                fe.target_symbol,
                fe.target_language,
                fe.edge_type,
                fe.protocol,
                fe.http_method,
                fe.url_pattern,
                fe.confidence,
                printf(',%d,', fe.id)
            FROM flow_edges fe
            JOIN files tf ON tf.id = fe.target_file_id
            WHERE tf.path = ?1
              AND (?2 = 0 OR fe.target_line = ?2)

            UNION ALL

            SELECT
                ft.depth + 1,
                fe.id,
                ft.edge_id,
                fe.source_file_id,
                fe.source_line,
                fe.source_symbol,
                fe.source_language,
                fe.target_file_id,
                fe.target_line,
                fe.target_symbol,
                fe.target_language,
                fe.edge_type,
                fe.protocol,
                fe.http_method,
                fe.url_pattern,
                fe.confidence,
                ft.edge_path || fe.id || ','
            FROM flow_trace ft
            JOIN flow_edges fe
              ON fe.target_file_id = ft.source_file_id
             AND (
                    (ft.source_line IS NOT NULL AND fe.target_line = ft.source_line)
                 OR (ft.source_line IS NULL AND ft.source_symbol IS NOT NULL
                     AND fe.target_symbol = ft.source_symbol)
             )
            WHERE ft.depth < ?3
              AND instr(ft.edge_path, printf(',%d,', fe.id)) = 0
        )
        SELECT DISTINCT
            ft.depth,
            ft.edge_id,
            ft.parent_edge_id,
            sf.path,
            ft.source_line,
            ft.source_symbol,
            COALESCE(ft.source_language, sf.language),
            tf.path,
            ft.target_line,
            ft.target_symbol,
            COALESCE(ft.target_language, tf.language),
            ft.edge_type,
            ft.protocol,
            ft.http_method,
            ft.url_pattern,
            ft.confidence
        FROM flow_trace ft
        JOIN files sf ON sf.id = ft.source_file_id
        LEFT JOIN files tf ON tf.id = ft.target_file_id
        ORDER BY ft.depth, ft.edge_id
    ";

    let mut stmt = conn
        .prepare(sql)
        .context("Failed to prepare trace_flow_reverse CTE")?;

    let steps = stmt
        .query_map(
            rusqlite::params![start_file, start_line, max_depth],
            flow_step_from_row,
        )
        .context("Failed to execute trace_flow_reverse CTE")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect trace_flow_reverse results")?;

    tracing::debug!(
        start_file,
        start_line,
        max_depth,
        steps = steps.len(),
        "trace_flow_reverse complete"
    );

    Ok(steps)
}

/// Forward and backward flow results for a single start node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BidirectionalFlow {
    /// Nodes reachable by following edges forward (source → target).
    pub forward: Vec<FlowStep>,
    /// Nodes reachable by following edges backward (target → source).
    pub backward: Vec<FlowStep>,
}

/// Run both `trace_flow` and `trace_flow_reverse` in a single call.
///
/// Useful when callers want to render the full context around a node without
/// making two round-trips to the database.
pub fn trace_flow_bidirectional(
    db: &Database,
    start_file: &str,
    start_line: u32,
    max_depth: u32,
) -> Result<BidirectionalFlow> {
    let forward = trace_flow(db, start_file, start_line, max_depth)?;
    let backward = trace_flow_reverse(db, start_file, start_line, max_depth)?;
    Ok(BidirectionalFlow { forward, backward })
}

/// Find all cross-language paths between two language boundaries.
///
/// Returns groups of `FlowStep` sequences — each inner `Vec<FlowStep>` is
/// one logical path from `source_language` to `target_language`, keyed by
/// the shared `url_pattern` or `edge_type`.
///
/// The implementation queries `flow_edges` directly for source → target
/// language transitions, then groups by `(url_pattern, edge_type)` to form
/// distinct paths.
pub fn cross_language_paths(
    db: &Database,
    source_language: &str,
    target_language: &str,
    limit: usize,
) -> Result<Vec<Vec<FlowStep>>> {
    let conn = db.conn();

    // Fetch direct cross-language edges.
    let sql = "
        SELECT
            0,
            fe.id,
            NULL,
            sf.path,
            fe.source_line,
            fe.source_symbol,
            COALESCE(fe.source_language, sf.language),
            tf.path,
            fe.target_line,
            fe.target_symbol,
            COALESCE(fe.target_language, tf.language),
            fe.edge_type,
            fe.protocol,
            fe.http_method,
            fe.url_pattern,
            fe.confidence
        FROM flow_edges fe
        JOIN files sf ON sf.id = fe.source_file_id
        JOIN files tf ON tf.id = fe.target_file_id
        WHERE fe.source_language = ?1
          AND fe.target_language = ?2
        ORDER BY fe.url_pattern, fe.edge_type, sf.path
        LIMIT ?3
    ";

    let effective_limit = if limit == 0 { 100 } else { limit };

    let mut stmt = conn
        .prepare(sql)
        .context("Failed to prepare cross_language_paths query")?;

    let rows: Vec<FlowStep> = stmt
        .query_map(
            rusqlite::params![source_language, target_language, effective_limit as i64],
            flow_step_from_row,
        )
        .context("Failed to execute cross_language_paths query")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect cross_language_paths rows")?;

    // Group direct paired hops by their channel key. Each FlowStep retains
    // both endpoints; no synthetic target-only row is needed.
    use std::collections::HashMap;

    let mut groups: HashMap<String, Vec<FlowStep>> = HashMap::new();

    for step in rows {
        let group_key = format!(
            "{}::{}::{}",
            step.edge_type,
            step.url_pattern.as_deref().unwrap_or(""),
            step.file_path
        );
        groups.entry(group_key).or_default().push(step);
    }

    let mut paths: Vec<Vec<FlowStep>> = groups.into_values().collect();
    // Sort for deterministic output in tests.
    paths.sort_by(|a, b| {
        let ak = a.first().map(|s| s.file_path.as_str()).unwrap_or("");
        let bk = b.first().map(|s| s.file_path.as_str()).unwrap_or("");
        ak.cmp(bk)
    });

    tracing::debug!(
        source_language,
        target_language,
        paths = paths.len(),
        "cross_language_paths complete"
    );

    Ok(paths)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "flow_tests.rs"]
mod tests;
