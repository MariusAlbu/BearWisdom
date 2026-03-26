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
    /// Distance from the start node (0 = origin).
    pub depth: u32,
    /// Absolute path of the file at this hop.
    pub file_path: String,
    /// Source line in that file, if known.
    pub line: Option<u32>,
    /// Symbol name at this hop, if known.
    pub symbol: Option<String>,
    /// Language tag (e.g. "typescript", "csharp").
    pub language: String,
    /// The kind of edge that led here (e.g. "http_call", "grpc_call").
    pub edge_type: String,
    /// Transport protocol if applicable (e.g. "http", "grpc").
    pub protocol: Option<String>,
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
    let conn = &db.conn;

    // The recursive CTE fans out from every edge that originates at
    // (start_file, start_line).  At each depth we follow all outbound edges
    // from the current node set.
    //
    // Cycle prevention: SQLite recursive CTEs will loop forever on cyclic
    // graphs unless we bound by depth.  The `WHERE ft.depth < ?3` bound in
    // the recursive term is our guard.
    let sql = "
        WITH RECURSIVE flow_trace(
            depth, file_id, line, symbol, language, edge_type, protocol
        ) AS (
            -- Base: all edges that leave start_file at start_line.
            SELECT
                0,
                fe.source_file_id,
                fe.source_line,
                fe.source_symbol,
                fe.source_language,
                fe.edge_type,
                fe.protocol
            FROM flow_edges fe
            JOIN files f ON f.id = fe.source_file_id
            WHERE f.path = ?1
              AND (?2 = 0 OR fe.source_line = ?2 OR fe.source_line IS NULL)

            UNION ALL

            -- Recursive: follow all edges out of the current set of nodes.
            SELECT
                ft.depth + 1,
                fe.target_file_id,
                fe.target_line,
                fe.target_symbol,
                fe.target_language,
                fe.edge_type,
                fe.protocol
            FROM flow_trace ft
            JOIN flow_edges fe ON fe.source_file_id = ft.file_id
            WHERE ft.depth < ?3
              AND fe.target_file_id IS NOT NULL
        )
        SELECT DISTINCT
            ft.depth,
            f.path,
            ft.line,
            ft.symbol,
            ft.language,
            ft.edge_type,
            ft.protocol
        FROM flow_trace ft
        JOIN files f ON f.id = ft.file_id
        ORDER BY ft.depth, f.path
    ";

    let mut stmt = conn.prepare(sql).context("Failed to prepare trace_flow CTE")?;

    let steps = stmt
        .query_map(
            rusqlite::params![start_file, start_line, max_depth],
            |row| {
                Ok(FlowStep {
                    depth: row.get::<_, u32>(0)?,
                    file_path: row.get::<_, String>(1)?,
                    line: row.get::<_, Option<u32>>(2)?,
                    symbol: row.get::<_, Option<String>>(3)?,
                    language: row.get::<_, String>(4).unwrap_or_default(),
                    edge_type: row.get::<_, String>(5)?,
                    protocol: row.get::<_, Option<String>>(6)?,
                })
            },
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
    let conn = &db.conn;

    // Fetch direct cross-language edges.
    let sql = "
        SELECT
            fe.id,
            sf.path  AS source_path,
            fe.source_line,
            fe.source_symbol,
            fe.source_language,
            fe.edge_type,
            fe.protocol,
            fe.url_pattern,
            tf.path  AS target_path,
            fe.target_line,
            fe.target_symbol,
            fe.target_language
        FROM flow_edges fe
        JOIN files sf ON sf.id = fe.source_file_id
        LEFT JOIN files tf ON tf.id = fe.target_file_id
        WHERE fe.source_language = ?1
          AND fe.target_language = ?2
        ORDER BY fe.url_pattern, fe.edge_type, sf.path
        LIMIT ?3
    ";

    let effective_limit = if limit == 0 { 100 } else { limit };

    let mut stmt = conn
        .prepare(sql)
        .context("Failed to prepare cross_language_paths query")?;

    // Each row becomes a two-step path: [source node → target node].
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        String, // source_path
        Option<u32>,
        Option<String>,
        String, // source_language
        String, // edge_type
        Option<String>, // protocol
        Option<String>, // url_pattern
        Option<String>, // target_path
        Option<u32>,
        Option<String>,
        String, // target_language
    )> = stmt
        .query_map(
            rusqlite::params![source_language, target_language, effective_limit as i64],
            |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<u32>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<u32>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, String>(11)?,
                ))
            },
        )
        .context("Failed to execute cross_language_paths query")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect cross_language_paths rows")?;

    // Build one two-step path per edge row.
    // Group by (url_pattern, edge_type) to merge duplicate edges.
    use std::collections::HashMap;

    let mut groups: HashMap<String, Vec<FlowStep>> = HashMap::new();

    for (
        source_path,
        source_line,
        source_symbol,
        source_language_val,
        edge_type,
        protocol,
        url_pattern,
        target_path,
        target_line,
        target_symbol,
        target_language_val,
    ) in rows
    {
        let group_key = format!(
            "{}::{}::{}",
            edge_type,
            url_pattern.as_deref().unwrap_or(""),
            source_path
        );

        let entry = groups.entry(group_key).or_default();

        // Only append the source step once per group.
        if entry.is_empty() {
            entry.push(FlowStep {
                depth: 0,
                file_path: source_path,
                line: source_line,
                symbol: source_symbol,
                language: source_language_val,
                edge_type: edge_type.clone(),
                protocol: protocol.clone(),
            });
        }

        // Always add the target step (there may be multiple targets per source).
        if let Some(tp) = target_path {
            entry.push(FlowStep {
                depth: 1,
                file_path: tp,
                line: target_line,
                symbol: target_symbol,
                language: target_language_val,
                edge_type: edge_type.clone(),
                protocol,
            });
        }
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
