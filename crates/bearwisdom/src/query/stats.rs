// =============================================================================
// query/stats.rs  —  index statistics queries
//
// Public functions for retrieving index health and size metrics.
// Replaces raw COUNT(*) queries scattered across CLI/web consumers.
// =============================================================================

use std::collections::{BTreeMap, HashMap};

use crate::db::Database;
use crate::query::QueryResult;
use crate::types::IndexStats;
use serde::{Deserialize, Serialize};

#[cfg(test)]
#[path = "stats_tests.rs"]
mod tests;

/// SQL WHERE-clause fragment that restricts `unresolved_refs` to *code*
/// references — excludes refs the resolution metric must not count.
///
/// Callers MUST alias `unresolved_refs` as `u` and `files` as `f` for the
/// fragment to bind. Composed via string concatenation; not parameterized
/// because every clause is a literal predicate over schema-fixed values.
///
/// Two patterns are excluded:
///
/// 1. `u.from_snippet = 1` — refs from Markdown fences and doctests. The
///    source text is sample code, not first-party project code.
/// 2. `(markdown|mdx) kind=imports` — Markdown link refs of the form
///    `[name](path/to/doc.md)`. The extractor emits these as Imports so
///    cross-document drift can be detected when the link target IS
///    indexed; when it isn't they fall to `unresolved_refs`. Document
///    cross-references are not code-resolution failures and must not
///    drag the rate down (Plan 01 — Resolution Gate).
pub(crate) const CODE_REF_FILTER: &str =
    "u.from_snippet = 0 \
     AND NOT (f.language IN ('markdown','mdx') AND u.kind = 'imports')";

/// Read index statistics from the database.
///
/// This is the canonical way to get counts — consumers should not issue
/// raw COUNT(*) queries against the tables.
pub fn index_stats(db: &Database) -> QueryResult<IndexStats> {
    let _timer = db.timer("index_stats");
    let conn = db.conn();
    // The internal `unresolved_ref_count` mirrors the resolution metric
    // and excludes doc cross-references via CODE_REF_FILTER. The external
    // count is a noise-tracking signal; it stays on the simple snippet
    // filter only.
    let internal_unresolved_sql = format!(
        "SELECT COUNT(*)
         FROM unresolved_refs u
         JOIN symbols s ON s.id = u.source_id
         JOIN files   f ON f.id = s.file_id
         WHERE s.origin = 'internal' AND {CODE_REF_FILTER}"
    );
    let combined_sql = format!(
        "SELECT
           (SELECT COUNT(*) FROM files WHERE origin = 'internal'),
           (SELECT COUNT(*) FROM symbols WHERE origin = 'internal'),
           (SELECT COUNT(*) FROM edges),
           ({internal_unresolved_sql}),
           (SELECT COUNT(*)
            FROM unresolved_refs ur
            JOIN symbols s ON s.id = ur.source_id
            WHERE ur.from_snippet = 0 AND s.origin = 'external'),
           (SELECT COUNT(*) FROM external_refs),
           (SELECT COUNT(*) FROM routes),
           (SELECT COUNT(*) FROM db_mappings),
           (SELECT COUNT(*) FROM flow_edges),
           (SELECT COUNT(*) FROM packages)"
    );
    let (
        file_count,
        symbol_count,
        edge_count,
        unresolved_ref_count,
        unresolved_ref_count_external,
        external_ref_count,
        route_count,
        db_mapping_count,
        flow_edge_count,
        package_count,
    ): (u32, u32, u32, u32, u32, u32, u32, u32, u32, u32) = conn.query_row(
        &combined_sql,
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
                r.get(9)?,
            ))
        },
    )?;

    Ok(IndexStats {
        file_count,
        symbol_count,
        edge_count,
        unresolved_ref_count,
        unresolved_ref_count_external,
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

/// One entry in the top-N unresolved-target worklist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnresolvedTarget {
    pub target_name: String,
    pub language: String,
    pub kind: String,
    pub count: u32,
}

/// Internal-only resolution breakdown for a single project.
///
/// Consumers (quality-check baseline, MCP `bw_diagnostics`, the resolution
/// gate report) use this as the authoritative picture of how well the
/// indexer understood the project's own code. All counts are restricted
/// to `files.origin = 'internal'` so external dependency noise
/// (node_modules, site-packages) never inflates the resolution rate.
///
/// The primary gate metric is `internal_resolution_rate`, defined per
/// `research/ArchitectureImprovements/Codex/01-resolution-gate-plan.md`:
///   internal_edges / (internal_edges + internal_unresolved) * 100
/// `resolution_rate` is the same value retained as a back-compat alias
/// for the older quality-check baselines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionBreakdown {
    /// Edges whose source symbol lives in a user file.
    pub internal_edges: u32,
    /// `unresolved_refs` whose source symbol lives in a user file (and
    /// which didn't come from a doc/markdown snippet).
    pub internal_unresolved: u32,
    /// Primary resolution gate metric, two decimals: internal_edges /
    /// (internal_edges + internal_unresolved) * 100. 100.0 when both
    /// sides are zero (empty project).
    pub internal_resolution_rate: f64,
    /// Back-compat alias for `internal_resolution_rate` — older callers
    /// (quality-check baselines, MCP wrappers) read `resolution_rate`.
    pub resolution_rate: f64,
    /// Map keyed `"<language>.<kind>"` (e.g. `"typescript.calls"`) →
    /// number of unresolved refs in user code of that language and kind.
    /// Pinpoints which extractor / resolver is leaking.
    pub unresolved_by_lang_kind: BTreeMap<String, u32>,
    /// Map keyed by source-symbol `origin_language` → unresolved count.
    /// Distinguishes refs originating in embedded sub-language regions
    /// (e.g. JS inside .vue/.svelte/.astro/.razor host files) from refs
    /// in plain host files. NULL `origin_language` rows roll up under
    /// the empty-string key.
    pub unresolved_by_origin_language: BTreeMap<String, u32>,
    /// Map keyed by `package_id` text repr → unresolved count. Empty
    /// string for refs whose source file has no package_id. Used to slice
    /// monorepos by workspace package.
    pub unresolved_by_package: BTreeMap<String, u32>,
    /// Map keyed by resolver `strategy` → resolved-edge count. Strategies
    /// reveal which resolution path is doing the heavy lifting and which
    /// have stagnated. NULL strategy rows roll up under empty-string.
    pub resolved_by_strategy: BTreeMap<String, u32>,
    /// Top-N unresolved (target_name, language, kind) tuples by count.
    /// The ordered worklist the gate report drives engineering off of.
    /// Capped at 25 entries.
    pub top_unresolved_targets: Vec<UnresolvedTarget>,
    /// Count of resolved edges with `confidence < low_confidence_threshold`.
    /// Heuristic-resolved edges are quality debt — they're not unresolved
    /// but they're not ground truth either.
    pub low_confidence_edges: u32,
    /// Threshold used for `low_confidence_edges`. Default 0.8 (matches
    /// `query::diagnostics::LOW_CONFIDENCE_THRESHOLD`).
    pub low_confidence_threshold: f64,
    /// Per-language file counts, user files only.
    pub languages: BTreeMap<String, u32>,
    /// Total persisted `code_chunks` rows — proxy for doc-drift coverage
    /// (markdown fences get chunked too).
    pub code_chunks: u32,
}

/// Compute the resolution breakdown for the currently-open index.
pub fn resolution_breakdown(db: &Database) -> QueryResult<ResolutionBreakdown> {
    let _timer = db.timer("resolution_breakdown");
    let conn = db.conn();

    let internal_edges: u32 = conn
        .query_row(
            "SELECT COUNT(*) FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.origin = 'internal'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let internal_unresolved_sql = format!(
        "SELECT COUNT(*)
         FROM unresolved_refs u
         JOIN symbols s ON s.id = u.source_id
         JOIN files   f ON f.id = s.file_id
         WHERE f.origin = 'internal' AND {CODE_REF_FILTER}"
    );
    let internal_unresolved: u32 = conn
        .query_row(&internal_unresolved_sql, [], |r| r.get(0))
        .unwrap_or(0);

    let mut languages: BTreeMap<String, u32> = BTreeMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT language, COUNT(*) FROM files
             WHERE origin = 'internal'
             GROUP BY language
             ORDER BY language",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))
        })?;
        for row in rows {
            let (lang, count) = row?;
            languages.insert(lang, count);
        }
    }

    let mut unresolved_by_lang_kind: BTreeMap<String, u32> = BTreeMap::new();
    {
        let by_lang_kind_sql = format!(
            "SELECT f.language, u.kind, COUNT(*)
             FROM unresolved_refs u
             JOIN symbols s ON s.id = u.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.origin = 'internal' AND {CODE_REF_FILTER}
             GROUP BY f.language, u.kind
             ORDER BY f.language, u.kind"
        );
        let mut stmt = conn.prepare(&by_lang_kind_sql)?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
            ))
        })?;
        for row in rows {
            let (lang, kind, count) = row?;
            unresolved_by_lang_kind.insert(format!("{lang}.{kind}"), count);
        }
    }

    // Unresolved by source-symbol origin_language (embedded-region slice).
    let mut unresolved_by_origin_language: BTreeMap<String, u32> = BTreeMap::new();
    {
        let by_origin_lang_sql = format!(
            "SELECT COALESCE(s.origin_language, '') AS ol, COUNT(*)
             FROM unresolved_refs u
             JOIN symbols s ON s.id = u.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.origin = 'internal' AND {CODE_REF_FILTER}
             GROUP BY ol
             ORDER BY ol"
        );
        let mut stmt = conn.prepare(&by_origin_lang_sql)?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))
        })?;
        for row in rows {
            let (ol, count) = row?;
            unresolved_by_origin_language.insert(ol, count);
        }
    }

    // Unresolved by package_id (monorepo slice).
    let mut unresolved_by_package: BTreeMap<String, u32> = BTreeMap::new();
    {
        let by_package_sql = format!(
            "SELECT COALESCE(CAST(f.package_id AS TEXT), '') AS pkg, COUNT(*)
             FROM unresolved_refs u
             JOIN symbols s ON s.id = u.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.origin = 'internal' AND {CODE_REF_FILTER}
             GROUP BY pkg
             ORDER BY pkg"
        );
        let mut stmt = conn.prepare(&by_package_sql)?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))
        })?;
        for row in rows {
            let (pkg, count) = row?;
            unresolved_by_package.insert(pkg, count);
        }
    }

    // Resolved edges grouped by strategy (which resolution path is firing).
    let mut resolved_by_strategy: BTreeMap<String, u32> = BTreeMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT COALESCE(e.strategy, '') AS strat, COUNT(*)
             FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.origin = 'internal'
             GROUP BY strat
             ORDER BY strat",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))
        })?;
        for row in rows {
            let (strat, count) = row?;
            resolved_by_strategy.insert(strat, count);
        }
    }

    // Top-N unresolved targets across languages — the ordered worklist
    // the resolution-gate report drives off of.
    const TOP_N_UNRESOLVED: u32 = 25;
    let mut top_unresolved_targets: Vec<UnresolvedTarget> = Vec::new();
    {
        let top_sql = format!(
            "SELECT u.target_name, f.language, u.kind, COUNT(*) AS c
             FROM unresolved_refs u
             JOIN symbols s ON s.id = u.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.origin = 'internal' AND {CODE_REF_FILTER}
             GROUP BY u.target_name, f.language, u.kind
             ORDER BY c DESC, u.target_name ASC
             LIMIT {TOP_N_UNRESOLVED}"
        );
        let mut stmt = conn.prepare(&top_sql)?;
        let rows = stmt.query_map([], |r| {
            Ok(UnresolvedTarget {
                target_name: r.get(0)?,
                language: r.get(1)?,
                kind: r.get(2)?,
                count: r.get(3)?,
            })
        })?;
        for row in rows {
            top_unresolved_targets.push(row?);
        }
    }

    // Heuristic-resolved edges count (confidence below threshold). Tracks
    // quality debt — these aren't unresolved, but they aren't ground truth.
    let low_confidence_threshold: f64 = crate::query::diagnostics::LOW_CONFIDENCE_THRESHOLD;
    let low_confidence_edges: u32 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM edges e
             JOIN symbols s ON s.id = e.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.origin = 'internal' AND e.confidence < ?1",
            [low_confidence_threshold],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let code_chunks: u32 = conn
        .query_row("SELECT COUNT(*) FROM code_chunks", [], |r| r.get(0))
        .unwrap_or(0);

    let resolution_rate = if internal_edges + internal_unresolved == 0 {
        100.0
    } else {
        (internal_edges as f64) * 100.0
            / (internal_edges as f64 + internal_unresolved as f64)
    };
    let resolution_rate = (resolution_rate * 100.0).round() / 100.0;

    Ok(ResolutionBreakdown {
        internal_edges,
        internal_unresolved,
        internal_resolution_rate: resolution_rate,
        resolution_rate,
        unresolved_by_lang_kind,
        unresolved_by_origin_language,
        unresolved_by_package,
        resolved_by_strategy,
        top_unresolved_targets,
        low_confidence_edges,
        low_confidence_threshold,
        languages,
        code_chunks,
    })
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
