// =============================================================================
// query/blast_radius.rs  —  N-hop dependency impact analysis
//
// "If I change symbol X, what else is affected?"
//
// Uses a recursive CTE (Common Table Expression) to walk the edge graph
// BACKWARDS from the target symbol.  Each hop goes one level up the
// dependency chain: if A calls B calls C, and we change C, then the blast
// radius of C includes B (depth=1) and A (depth=2).
//
// Terminology:
//   • center   — the symbol being changed
//   • affected — every symbol that (transitively) depends on the center
//   • depth    — number of hops from the center (1 = direct caller)
// =============================================================================

use crate::db::Database;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::query::architecture::SymbolSummary;

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// A single symbol that would be affected if the center symbol changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectedSymbol {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file_path: String,
    /// How many hops away from the center symbol (1 = direct dependent).
    pub depth: u32,
    /// The kind of edge connecting this symbol to its predecessor in the path.
    pub edge_kind: String,
}

/// The complete blast radius result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlastRadiusResult {
    /// The symbol that was the starting point of the analysis.
    pub center: SymbolSummary,
    /// All symbols reachable within `max_depth` hops, excluding the center.
    pub affected: Vec<AffectedSymbol>,
    /// Total number of affected symbols found (may be < affected.len() if
    /// duplicates were collapsed by DISTINCT).
    pub total_affected: u32,
    /// The maximum depth actually found (may be less than the requested limit).
    pub max_depth: u32,
    /// True when the result set was capped by `max_results`.
    /// MCP/CLI consumers should surface this to the caller.
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// Public function
// ---------------------------------------------------------------------------

/// Compute the blast radius of `symbol_name` up to `max_depth` hops.
///
/// `symbol_name` may be a simple name or a fully-qualified name.
/// When the name is ambiguous (multiple symbols share it), the first match
/// by qualified_name alphabetically is used.
///
/// `max_results` caps the number of rows returned by the final SELECT.
/// When the cap is hit, `BlastRadiusResult::truncated` is set to `true`.
/// Pass `500` as a safe default; use a lower value for UI previews.
///
/// Returns `Ok(None)` if the symbol is not found in the index.
pub fn blast_radius(
    db: &Database,
    symbol_name: &str,
    max_depth: u32,
    max_results: u32,
) -> Result<Option<BlastRadiusResult>> {
    let _timer = db.timer("blast_radius");
    let conn = &db.conn;

    // --- Resolve the symbol to an id + summary ---
    // Try exact qualified name first, then simple name fallback.
    // For ambiguous names, prefer the symbol with the most incoming edges —
    // that's the most "depended-on" symbol and the most useful blast-radius center.
    let lookup_sql = if symbol_name.contains('.') {
        "SELECT s.id, s.name, s.qualified_name, s.kind, f.path, s.line
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE s.qualified_name = ?1
         LIMIT 1"
    } else {
        "SELECT s.id, s.name, s.qualified_name, s.kind, f.path, s.line
         FROM symbols s
         JOIN files f ON f.id = s.file_id
         LEFT JOIN edges e ON e.target_id = s.id
         WHERE s.name = ?1
         GROUP BY s.id
         ORDER BY COUNT(e.target_id) DESC
         LIMIT 1"
    };

    // query_row returns Err(QueryReturnedNoRows) if nothing is found.
    let center_result = conn.query_row(lookup_sql, [symbol_name], |row| {
        Ok((
            row.get::<_, i64>(0)?,        // id
            SymbolSummary {
                name:           row.get(1)?,
                qualified_name: row.get(2)?,
                kind:           row.get(3)?,
                file_path:      row.get(4)?,
                line:           row.get(5)?,
            },
        ))
    });

    let (center_id, center) = match center_result {
        Ok(pair) => pair,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => return Err(e).context("Failed to look up center symbol"),
    };

    // --- Recursive CTE: walk backwards through the edge graph ---
    //
    // The CTE starts from the center symbol and follows edges in the
    // REVERSE direction (source → target means "source depends on target",
    // so we follow source_id to find all dependents of the center).
    //
    // UNION deduplicates on the full row (id, depth, edge_kind), which means
    // the same symbol can appear at multiple depths via different paths,
    // creating cycles.  We let the CTE run (bounded by max_depth) and then
    // use ROW_NUMBER() to keep only the first (shallowest) occurrence of
    // each symbol in the output.
    //
    // LIMIT ?3 caps the output row count.  When the cap is reached, the caller
    // sets BlastRadiusResult::truncated so consumers know the list is partial.
    let sql = "
        WITH RECURSIVE blast(id, depth, edge_kind) AS (
            -- Seed: the center symbol itself at depth 0.
            SELECT ?1 AS id, 0 AS depth, '' AS edge_kind

            UNION

            -- Recursive step: find every symbol that has an edge pointing TO
            -- a symbol already in our blast set.
            SELECT e.source_id,
                   blast.depth + 1,
                   e.kind
            FROM edges e
            JOIN blast ON blast.id = e.target_id
            WHERE blast.depth < ?2
        )
        SELECT s.name, s.qualified_name, s.kind,
               f.path AS file_path, sub.depth, sub.edge_kind
        FROM (
            SELECT id, depth, edge_kind,
                   ROW_NUMBER() OVER (PARTITION BY id ORDER BY depth) AS rn
            FROM blast
            WHERE depth > 0
        ) sub
        JOIN symbols s ON s.id = sub.id
        JOIN files   f ON f.id = s.file_id
        WHERE sub.rn = 1
        ORDER BY sub.depth, f.path
        LIMIT ?3
    ";

    let mut stmt = conn.prepare(sql).context("Failed to prepare blast radius CTE")?;

    let rows = stmt
        .query_map(rusqlite::params![center_id, max_depth, max_results], |row| {
            Ok(AffectedSymbol {
                name:           row.get(0)?,
                qualified_name: row.get(1)?,
                kind:           row.get(2)?,
                file_path:      row.get(3)?,
                depth:          row.get(4)?,
                edge_kind:      row.get(5)?,
            })
        })
        .context("Failed to execute blast radius query")?;

    let affected: Vec<AffectedSymbol> = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect blast radius rows")?;

    let truncated = (affected.len() as u32) >= max_results;
    let total_affected = affected.len() as u32;
    let max_depth_found = affected.iter().map(|a| a.depth).max().unwrap_or(0);

    Ok(Some(BlastRadiusResult {
        center,
        affected,
        total_affected,
        max_depth: max_depth_found,
        truncated,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "blast_radius_tests.rs"]
mod tests;
