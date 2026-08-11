// =============================================================================
// query/flow_stats — flow-edge listing and diagnostics reporting
//
// Row-level flow-edge dumps and the pairing/bucketed diagnostics view over
// flow_edges. Aggregate index counts stay in query/stats.
// =============================================================================

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::query::error::QueryResult;

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
                source_file: row.get(0)?,
                source_line: row.get(1)?,
                source_symbol: row.get(2)?,
                source_language: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                target_file: row.get(4)?,
                target_line: row.get(5)?,
                target_symbol: row.get(6)?,
                target_language: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
                edge_type: row.get(8)?,
                protocol: row.get(9)?,
                url_pattern: row.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(FlowEdgesData {
        edges,
        total,
        by_edge_type,
        by_language_pair,
    })
}

// ---------------------------------------------------------------------------
// Flow diagnostics — paired vs single-ended breakdown
// ---------------------------------------------------------------------------

/// Pairing counts for one bucket (edge_type / protocol / language /
/// edge_type+language slice). Single-ended rows are `flow_edges` whose
/// `target_file_id IS NULL` — the resolver-side equivalent of an
/// unresolved reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowPairing {
    pub paired: u32,
    pub single_ended: u32,
}

impl FlowPairing {
    /// Total edges in this bucket.
    pub fn total(&self) -> u32 {
        self.paired + self.single_ended
    }

    /// `paired / total * 100`, capped to two decimals. 100.0 when the bucket
    /// is empty so empty buckets don't show up as 0% pairing.
    pub fn pairing_rate(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            return 100.0;
        }
        let rate = (self.paired as f64) / (total as f64) * 100.0;
        (rate * 100.0).round() / 100.0
    }
}

/// One (edge_type, protocol) bucket in the diagnostic breakdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowEdgeTypeBucket {
    pub edge_type: String,
    pub protocol: Option<String>,
    pub paired: u32,
    pub single_ended: u32,
    /// `paired / total * 100`, two decimals.
    pub pairing_rate: f64,
}

/// One example single-ended group surfaced in the diagnostic worklist:
/// a (edge_type, protocol, url_pattern, source_language) tuple with its
/// count and an example file/line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleEndedExample {
    pub edge_type: String,
    pub protocol: Option<String>,
    pub url_pattern: Option<String>,
    pub source_language: Option<String>,
    pub example_file: Option<String>,
    pub example_line: Option<u32>,
    pub count: u32,
}

/// Full pairing-quality report for `flow_edges`.
///
/// The connector equivalent of `ResolutionBreakdown`: every produced flow
/// edge is either paired (both endpoints resolved across files) or
/// single-ended (only the producer or only the consumer side was emitted).
/// Pairing quality is the gate metric for the connector-kill — a healthy
/// project pairs the bulk of its emitted HTTP / RPC / WebSocket / queue
/// edges across services and only leaves single-ended rows where the
/// counterpart genuinely lives outside the indexed code.
///
/// See `research/ArchitectureImprovements/Codex/04-flow-connectors-plan.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowDiagnostics {
    /// Total `flow_edges` rows.
    pub total: u32,
    /// Rows with both ends resolved (`target_file_id IS NOT NULL`).
    pub paired: u32,
    /// Rows with the target side unresolved (`target_file_id IS NULL`).
    pub single_ended: u32,
    /// `paired / total * 100`, two decimals. 100.0 for empty indexes.
    pub pairing_rate: f64,
    /// Per-(edge_type, protocol) breakdown sorted by single_ended desc, then
    /// total desc. The worklist headline: which connector kinds are leaking.
    pub by_edge_type: Vec<FlowEdgeTypeBucket>,
    /// Per source-language pairing. Pinpoints whether a specific language's
    /// resolver is the source of unpaired edges.
    pub by_source_language: BTreeMap<String, FlowPairing>,
    /// Top single-ended examples — (edge_type, protocol, url_pattern,
    /// source_language) groups with the highest count, capped at 25. Each
    /// row carries an arbitrary example file/line for the user to inspect.
    pub top_single_ended: Vec<SingleEndedExample>,
}

/// Compute pairing-quality diagnostics for the project's `flow_edges`.
///
/// Single-ended rows = `target_file_id IS NULL`. They're the post-Phase H
/// equivalent of "unmatched starts/stops" — the connector kill emitted them
/// but the pairer never found a partner. This report breaks the count down
/// so the next round of resolver work can target the heaviest leak first.
pub fn flow_diagnostics(db: &Database) -> QueryResult<FlowDiagnostics> {
    let _timer = db.timer("flow_diagnostics");
    let conn = db.conn();

    // --- Headline totals + (edge_type, protocol) buckets in one scan.
    let mut by_edge_type: Vec<FlowEdgeTypeBucket> = Vec::new();
    let mut total = 0u32;
    let mut paired_total = 0u32;
    let mut single_total = 0u32;
    {
        let mut stmt = conn.prepare(
            "SELECT edge_type,
                    protocol,
                    SUM(CASE WHEN target_file_id IS NULL THEN 0 ELSE 1 END) AS paired,
                    SUM(CASE WHEN target_file_id IS NULL THEN 1 ELSE 0 END) AS single_ended
             FROM flow_edges
             GROUP BY edge_type, protocol",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let edge_type: String = row.get(0)?;
            let protocol: Option<String> = row.get(1)?;
            let paired: u32 = row.get::<_, i64>(2)? as u32;
            let single_ended: u32 = row.get::<_, i64>(3)? as u32;
            paired_total += paired;
            single_total += single_ended;
            total += paired + single_ended;
            let pairing = FlowPairing {
                paired,
                single_ended,
            };
            by_edge_type.push(FlowEdgeTypeBucket {
                edge_type,
                protocol,
                paired,
                single_ended,
                pairing_rate: pairing.pairing_rate(),
            });
        }
    }
    by_edge_type.sort_by(|a, b| {
        b.single_ended
            .cmp(&a.single_ended)
            .then_with(|| (b.paired + b.single_ended).cmp(&(a.paired + a.single_ended)))
            .then_with(|| a.edge_type.cmp(&b.edge_type))
    });

    let pairing_rate = if total == 0 {
        100.0
    } else {
        let r = (paired_total as f64) / (total as f64) * 100.0;
        (r * 100.0).round() / 100.0
    };

    // --- Per-source-language pairing.
    let mut by_source_language: BTreeMap<String, FlowPairing> = BTreeMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT COALESCE(fe.source_language, sf.language, '') AS lang,
                    SUM(CASE WHEN fe.target_file_id IS NULL THEN 0 ELSE 1 END) AS paired,
                    SUM(CASE WHEN fe.target_file_id IS NULL THEN 1 ELSE 0 END) AS single_ended
             FROM flow_edges fe
             JOIN files sf ON sf.id = fe.source_file_id
             GROUP BY lang",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let lang: String = row.get(0)?;
            let paired: u32 = row.get::<_, i64>(1)? as u32;
            let single_ended: u32 = row.get::<_, i64>(2)? as u32;
            by_source_language.insert(
                lang,
                FlowPairing {
                    paired,
                    single_ended,
                },
            );
        }
    }

    // --- Top-N single-ended worklist.
    //
    // Grouping by (edge_type, protocol, url_pattern, source_language) keeps
    // duplicate emissions (same URL emitted at 12 different call sites)
    // collapsed into one worklist entry. The example_file/line picks any
    // representative row via MIN.
    let mut top_single_ended: Vec<SingleEndedExample> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT fe.edge_type,
                    fe.protocol,
                    fe.url_pattern,
                    COALESCE(fe.source_language, sf.language) AS lang,
                    MIN(sf.path) AS example_file,
                    MIN(fe.source_line) AS example_line,
                    COUNT(*) AS cnt
             FROM flow_edges fe
             JOIN files sf ON sf.id = fe.source_file_id
             WHERE fe.target_file_id IS NULL
             GROUP BY fe.edge_type, fe.protocol, fe.url_pattern, lang
             ORDER BY cnt DESC, fe.edge_type, fe.url_pattern
             LIMIT 25",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            top_single_ended.push(SingleEndedExample {
                edge_type: row.get(0)?,
                protocol: row.get(1)?,
                url_pattern: row.get(2)?,
                source_language: row.get(3)?,
                example_file: row.get(4)?,
                example_line: row.get::<_, Option<i64>>(5)?.map(|v| v as u32),
                count: row.get::<_, i64>(6)? as u32,
            });
        }
    }

    Ok(FlowDiagnostics {
        total,
        paired: paired_total,
        single_ended: single_total,
        pairing_rate,
        by_edge_type,
        by_source_language,
        top_single_ended,
    })
}
