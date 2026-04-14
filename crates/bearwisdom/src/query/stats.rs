// =============================================================================
// query/stats.rs  —  index statistics queries
//
// Public functions for retrieving index health and size metrics.
// Replaces raw COUNT(*) queries scattered across CLI/web consumers.
// =============================================================================

use std::collections::HashMap;

use crate::db::Database;
use crate::query::QueryResult;
use crate::types::IndexStats;
use serde::{Deserialize, Serialize};

/// Read index statistics from the database.
///
/// This is the canonical way to get counts — consumers should not issue
/// raw COUNT(*) queries against the tables.
pub fn index_stats(db: &Database) -> QueryResult<IndexStats> {
    let _timer = db.timer("index_stats");
    let conn = db.conn();
    let (
        file_count,
        symbol_count,
        edge_count,
        unresolved_ref_count,
        external_ref_count,
        route_count,
        db_mapping_count,
        flow_edge_count,
        package_count,
    ): (u32, u32, u32, u32, u32, u32, u32, u32, u32) = conn.query_row(
        "SELECT
           (SELECT COUNT(*) FROM files WHERE origin = 'internal'),
           (SELECT COUNT(*) FROM symbols WHERE origin = 'internal'),
           (SELECT COUNT(*) FROM edges),
           (SELECT COUNT(*) FROM unresolved_refs WHERE from_snippet = 0),
           (SELECT COUNT(*) FROM external_refs),
           (SELECT COUNT(*) FROM routes),
           (SELECT COUNT(*) FROM db_mappings),
           (SELECT COUNT(*) FROM flow_edges),
           (SELECT COUNT(*) FROM packages)",
        [],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
            ))
        },
    )?;

    Ok(IndexStats {
        file_count,
        symbol_count,
        edge_count,
        unresolved_ref_count,
        external_ref_count,
        route_count,
        db_mapping_count,
        flow_edge_count,
        package_count,
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

/// Count connection_points with direction='start' that have no matching flow_edge.
pub fn unresolved_flow_count(db: &Database) -> QueryResult<u32> {
    let _timer = db.timer("unresolved_flow_count");
    let count: u32 = db.conn().query_row(
        "SELECT COUNT(*) FROM connection_points cp
         WHERE cp.direction = 'start'
           AND NOT EXISTS (
               SELECT 1 FROM flow_edges fe
               WHERE fe.source_file_id = cp.file_id
                 AND fe.source_line    = cp.line
           )",
        [],
        |r| r.get(0),
    )?;
    Ok(count)
}

/// Count flow edges of a specific type.
pub fn flow_edge_count_by_type(db: &Database, edge_type: &str) -> QueryResult<u32> {
    let _timer = db.timer("flow_edge_count_by_type");
    let count: u32 = db
        .query_row(
            "SELECT COUNT(*) FROM flow_edges WHERE edge_type = ?1",
            [edge_type],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Ok(count)
}

/// Get flow edge counts grouped by edge_type.
pub fn flow_edge_breakdown(db: &Database) -> QueryResult<Vec<FlowEdgeBreakdown>> {
    let _timer = db.timer("flow_edge_breakdown");
    let conn = db.conn();
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

/// Return the number of concepts currently in the index.
pub fn concept_count(db: &Database) -> QueryResult<u32> {
    let _timer = db.timer("concept_count");
    let count: u32 = db
        .query_row("SELECT COUNT(*) FROM concepts", [], |r| r.get(0))
        .unwrap_or(0);
    Ok(count)
}

/// A single flow edge row returned by [`flow_edges_data`].
#[derive(Debug, Serialize, Deserialize)]
pub struct FlowEdgeRow {
    pub source_file: Option<String>,
    pub source_line: Option<i64>,
    pub source_symbol: Option<String>,
    pub source_language: String,
    pub target_file: Option<String>,
    pub target_line: Option<i64>,
    pub target_symbol: Option<String>,
    pub target_language: String,
    pub edge_type: String,
    pub protocol: Option<String>,
    pub url_pattern: Option<String>,
}

/// Aggregated flow edge data: a sample of `limit` rows interleaved by type,
/// plus summary counts by edge type and language pair.
#[derive(Debug, Serialize, Deserialize)]
pub struct FlowEdgesData {
    pub edges: Vec<FlowEdgeRow>,
    pub total: u32,
    pub by_edge_type: HashMap<String, u32>,
    pub by_language_pair: HashMap<String, u32>,
}

/// Query flow edge data with per-type interleaving so the `limit` sample is
/// representative across all edge types.
///
/// Builds summary counts over the full dataset first, then fetches the
/// interleaved sample.
pub fn flow_edges_data(db: &Database, limit: usize) -> QueryResult<FlowEdgesData> {
    let _timer = db.timer("flow_edges_data");
    let conn = db.conn();

    // Summary counts from the full dataset (before limit).
    let mut by_edge_type: HashMap<String, u32> = HashMap::new();
    let mut by_language_pair: HashMap<String, u32> = HashMap::new();
    let total: u32 = {
        let mut stmt = conn.prepare(
            "SELECT fe.edge_type,
                    COALESCE(fe.source_language, sf.language, '') AS src_lang,
                    COALESCE(fe.target_language, tf.language, '') AS tgt_lang,
                    COUNT(*) AS cnt
             FROM flow_edges fe
             JOIN files sf ON sf.id = fe.source_file_id
             LEFT JOIN files tf ON tf.id = fe.target_file_id
             GROUP BY fe.edge_type, src_lang, tgt_lang",
        )?;
        let mut total = 0u32;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let et: String = row.get(0)?;
            let src: String = row.get::<_, Option<String>>(1)?.unwrap_or_default();
            let tgt: String = row.get::<_, Option<String>>(2)?.unwrap_or_default();
            let cnt: u32 = row.get(3)?;
            *by_edge_type.entry(et).or_default() += cnt;
            let pair = format!("{src} \u{2192} {tgt}");
            *by_language_pair.entry(pair).or_default() += cnt;
            total += cnt;
        }
        total
    };

    // Interleave edge types so the limit gets a fair mix.
    let mut stmt = conn.prepare(
        "SELECT source_file, source_line, source_symbol, source_language,
                target_file, target_line, target_symbol, target_language,
                edge_type, protocol, url_pattern
         FROM (
             SELECT
                 sf.path                                       AS source_file,
                 fe.source_line,
                 fe.source_symbol,
                 COALESCE(fe.source_language, sf.language, '') AS source_language,
                 tf.path                                       AS target_file,
                 fe.target_line,
                 fe.target_symbol,
                 COALESCE(fe.target_language, tf.language, '') AS target_language,
                 fe.edge_type,
                 fe.protocol,
                 fe.url_pattern,
                 ROW_NUMBER() OVER (PARTITION BY fe.edge_type ORDER BY sf.path, fe.source_line) AS rn
             FROM flow_edges fe
             JOIN files sf ON sf.id = fe.source_file_id
             LEFT JOIN files tf ON tf.id = fe.target_file_id
         )
         ORDER BY rn, edge_type
         LIMIT ?1",
    )?;

    let edges = stmt
        .query_map([limit as i64], |row| {
            Ok(FlowEdgeRow {
                source_file:     row.get(0)?,
                source_line:     row.get(1)?,
                source_symbol:   row.get(2)?,
                source_language: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                target_file:     row.get(4)?,
                target_line:     row.get(5)?,
                target_symbol:   row.get(6)?,
                target_language: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
                edge_type:       row.get(8)?,
                protocol:        row.get(9)?,
                url_pattern:     row.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(FlowEdgesData { edges, total, by_edge_type, by_language_pair })
}

/// List all HTTP routes from the index.
pub fn list_routes(db: &Database) -> QueryResult<Vec<crate::types::RouteInfo>> {
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT r.id, f.path, r.http_method, r.route_template, r.resolved_route,
                r.line, s.name
         FROM routes r
         JOIN files f ON r.file_id = f.id
         LEFT JOIN symbols s ON r.symbol_id = s.id
         ORDER BY r.http_method, r.route_template",
    )?;

    let rows = stmt
        .query_map([], |row| {
            Ok(crate::types::RouteInfo {
                id: row.get(0)?,
                file_path: row.get(1)?,
                http_method: row.get(2)?,
                route_template: row.get(3)?,
                resolved_route: row.get(4)?,
                line: row.get::<_, Option<u32>>(5)?.unwrap_or(0),
                handler_name: row.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(rows)
}
