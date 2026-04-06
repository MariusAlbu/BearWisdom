// =============================================================================
// query/stats.rs  —  index statistics queries
//
// Public functions for retrieving index health and size metrics.
// Replaces raw COUNT(*) queries scattered across CLI/web consumers.
// =============================================================================

use crate::db::Database;
use crate::types::IndexStats;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Read index statistics from the database.
///
/// This is the canonical way to get counts — consumers should not issue
/// raw COUNT(*) queries against the tables.
pub fn index_stats(db: &Database) -> Result<IndexStats> {
    let _timer = db.timer("index_stats");
    let conn = &db.conn;
    let file_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
    let symbol_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))?;
    let edge_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
    let unresolved_ref_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM unresolved_refs", [], |r| r.get(0))?;
    let external_ref_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM external_refs", [], |r| r.get(0))?;
    let route_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM routes", [], |r| r.get(0))?;
    let db_mapping_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM db_mappings", [], |r| r.get(0))?;
    let flow_edge_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM flow_edges", [], |r| r.get(0))?;

    Ok(IndexStats {
        file_count,
        symbol_count,
        edge_count,
        unresolved_ref_count,
        external_ref_count,
        route_count,
        db_mapping_count,
        flow_edge_count,
        files_with_errors: 0,
        duration_ms: 0,
    })
}

/// A flow edge type with its count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowEdgeBreakdown {
    pub edge_type: String,
    pub count: u32,
}

/// Get flow edge counts grouped by edge_type.
pub fn flow_edge_breakdown(db: &Database) -> Result<Vec<FlowEdgeBreakdown>> {
    let _timer = db.timer("flow_edge_breakdown");
    let conn = &db.conn;
    let mut stmt = conn.prepare(
        "SELECT edge_type, COUNT(*) FROM flow_edges GROUP BY edge_type ORDER BY COUNT(*) DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(FlowEdgeBreakdown {
            edge_type: r.get(0)?,
            count: r.get(1)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}
