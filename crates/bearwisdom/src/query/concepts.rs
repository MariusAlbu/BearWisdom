// =============================================================================
// query/concepts.rs  —  domain concept management and auto-discovery
//
// A "concept" is a named domain grouping (e.g. "authentication", "catalog",
// "order-processing").  Symbols can belong to one or more concepts either:
//   • Manually — a user explicitly assigns a symbol to a concept.
//   • Automatically — the concept has an `auto_pattern` (a prefix or glob)
//     and any symbol whose qualified_name starts with that prefix is auto-
//     assigned.
//
// `discover_concepts`:
//   Scans all qualified names in the index, extracts the top-level namespace
//   segments (the first two dot-separated components), and creates a concept
//   entry for each distinct namespace prefix.  E.g. a qualified name like
//   "Microsoft.eShop.Catalog.CatalogService" yields prefix "Microsoft.eShop"
//   with auto_pattern "Microsoft.eShop.*".
//
// `auto_assign_concepts`:
//   For every concept that has an auto_pattern, finds all symbols whose
//   qualified_name starts with the pattern prefix (stripping the trailing ".*")
//   and inserts them into concept_members.  Idempotent — uses INSERT OR IGNORE.
//
// `concept_subgraph`:
//   Expands a concept's member set by up to `max_depth` hops through the edge
//   graph, returning the resulting subgraph (nodes + edges).
// =============================================================================

use crate::db::Database;
use crate::query::architecture::SymbolSummary;
use crate::query::subgraph::SubgraphResult;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// A lightweight concept summary returned by `list_concepts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConceptSummary {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub auto_pattern: Option<String>,
    /// Number of symbols that belong to this concept (auto + manual).
    pub member_count: u32,
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Auto-assign symbols to all concepts that have an `auto_pattern`.
///
/// The pattern is treated as a qualified-name prefix: a concept with
/// `auto_pattern = "eShop.Catalog.*"` will include every symbol whose
/// `qualified_name` starts with `"eShop.Catalog."`.
///
/// Returns the number of new memberships inserted (already-existing ones are
/// silently skipped).
pub fn auto_assign_concepts(db: &Database) -> Result<u32> {
    let _timer = db.timer("auto_assign_concepts");
    let conn = &db.conn;

    // Fetch all concepts that have an auto_pattern.
    let patterns: Vec<(i64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, auto_pattern FROM concepts WHERE auto_pattern IS NOT NULL"
        ).context("Failed to prepare concept pattern query")?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        }).context("Failed to execute concept pattern query")?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect concept patterns")?
    };

    let mut inserted_total = 0u32;

    for (concept_id, pattern) in &patterns {
        // Strip the trailing ".*" suffix to get the prefix for LIKE matching.
        // E.g. "eShop.Catalog.*" → "eShop.Catalog."
        let prefix = if let Some(p) = pattern.strip_suffix(".*") {
            format!("{p}.")
        } else {
            // If no wildcard, treat the whole pattern as a prefix.
            format!("{pattern}.")
        };

        // Find all symbols whose qualified_name starts with this prefix.
        // INSERT OR IGNORE skips symbols that are already assigned.
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO concept_members (concept_id, symbol_id, auto_assigned)
             SELECT ?1, id, 1
             FROM symbols
             WHERE qualified_name LIKE ?2 ESCAPE '\\'",
            rusqlite::params![concept_id, format!("{prefix}%")],
        ).with_context(|| format!("Failed to auto-assign concept {concept_id}"))?;

        inserted_total += inserted as u32;
    }

    Ok(inserted_total)
}

/// Return a summary of every concept in the index, ordered by name.
pub fn list_concepts(db: &Database) -> Result<Vec<ConceptSummary>> {
    let _timer = db.timer("list_concepts");
    let conn = &db.conn;

    let mut stmt = conn.prepare(
        "SELECT c.id,
                c.name,
                c.description,
                c.auto_pattern,
                COUNT(cm.symbol_id) AS member_count
         FROM concepts c
         LEFT JOIN concept_members cm ON cm.concept_id = c.id
         GROUP BY c.id
         ORDER BY c.name",
    ).context("Failed to prepare list_concepts query")?;

    let rows = stmt.query_map([], |row| {
        Ok(ConceptSummary {
            id:           row.get(0)?,
            name:         row.get(1)?,
            description:  row.get(2)?,
            auto_pattern: row.get(3)?,
            member_count: row.get(4)?,
        })
    }).context("Failed to execute list_concepts query")?;

    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect concept list")
}

/// Return all symbols that belong to `concept_name`, up to `limit` results.
///
/// Returns an empty vec if the concept does not exist or has no members.
pub fn concept_members(
    db: &Database,
    concept_name: &str,
    limit: usize,
) -> Result<Vec<SymbolSummary>> {
    let _timer = db.timer("concept_members");
    let conn = &db.conn;

    let limit_clause = if limit > 0 { format!("LIMIT {limit}") } else { String::new() };

    let sql = format!(
        "SELECT s.name, s.qualified_name, s.kind, f.path, s.line
         FROM concept_members cm
         JOIN concepts c  ON c.id  = cm.concept_id
         JOIN symbols  s  ON s.id  = cm.symbol_id
         JOIN files    f  ON f.id  = s.file_id
         WHERE c.name = ?1
         ORDER BY s.qualified_name
         {limit_clause}"
    );

    let mut stmt = conn.prepare(&sql)
        .context("Failed to prepare concept_members query")?;

    let rows = stmt.query_map([concept_name], |row| {
        Ok(SymbolSummary {
            name:           row.get(0)?,
            qualified_name: row.get(1)?,
            kind:           row.get(2)?,
            file_path:      row.get(3)?,
            line:           row.get(4)?,
        })
    }).context("Failed to execute concept_members query")?;

    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect concept members")
}

/// Build a subgraph of all symbols in `concept_name` plus any symbols
/// reachable from them within `max_depth` hops through the edge graph.
///
/// Returns the expanded subgraph as nodes + edges.  Returns an empty
/// subgraph if the concept does not exist or has no members.
pub fn concept_subgraph(
    db: &Database,
    concept_name: &str,
    max_depth: u32,
) -> Result<SubgraphResult> {
    let _timer = db.timer("concept_subgraph");
    // Quick check: does the concept have any members?
    // We use this to avoid running the heavier export_graph when there is nothing to return.
    let conn = &db.conn;
    let member_count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM concept_members cm
         JOIN concepts c ON c.id = cm.concept_id
         WHERE c.name = ?1",
        [concept_name],
        |r| r.get(0),
    ).context("Failed to count concept members for subgraph")?;

    if member_count == 0 {
        return Ok(SubgraphResult { nodes: vec![], edges: vec![] });
    }

    // A generous node cap: max_depth * 1000 or at least 500.
    let node_cap = (max_depth as usize * 1000).max(500);

    // Use the subgraph exporter with the "@concept_name" filter.
    crate::query::subgraph::export_graph(
        db,
        Some(&format!("@{concept_name}")),
        node_cap,
    )
}

/// Auto-discover concepts by analyzing namespace structure.
///
/// Groups all symbols by the first two segments of their qualified name
/// (e.g. "Microsoft.eShop") and creates a concept for each distinct prefix
/// that does not already exist.  Sets `auto_pattern = "{prefix}.*"`.
///
/// Returns the names of all concepts created (not including pre-existing ones).
pub fn discover_concepts(db: &Database) -> Result<Vec<String>> {
    let _timer = db.timer("discover_concepts");
    let conn = &db.conn;

    // Extract the first-two-segment prefix from every qualified_name that
    // has at least 3 dot-separated components.
    //
    // SQLite string functions used:
    //   instr(s, '.') → position of first '.'  (1-based, 0 if not found)
    //   substr(s, start, len) → substring
    //
    // For a qualified_name like "Microsoft.eShop.Catalog.Service":
    //   first_dot  = instr(qn, '.')          → 10  (position of first '.')
    //   second_seg = substr(qn, first_dot+1) → "eShop.Catalog.Service"
    //   second_dot = instr(second_seg, '.')  → 6   (position of '.' in second_seg)
    //   prefix     = substr(qn, 1, first_dot + second_dot - 1) → "Microsoft.eShop"
    //
    // The WHERE clause ensures we only process names with at least two dots
    // (three components).
    let prefixes: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT
                 substr(qualified_name, 1,
                     instr(qualified_name, '.') +
                     instr(substr(qualified_name, instr(qualified_name, '.') + 1), '.') - 1
                 ) AS prefix
             FROM symbols
             WHERE qualified_name LIKE '%.%.%'",
        ).context("Failed to prepare prefix discovery query")?;

        let rows = stmt.query_map([], |row| row.get::<_, String>(0))
            .context("Failed to execute prefix discovery query")?;

        rows.filter_map(|r| r.ok())
            .filter(|p| !p.is_empty() && p.contains('.'))
            .collect()
    };

    let mut created: Vec<String> = Vec::new();

    for prefix in &prefixes {
        let auto_pattern = format!("{prefix}.*");

        // Insert only if the concept doesn't already exist.
        let rows_affected = conn.execute(
            "INSERT OR IGNORE INTO concepts (name, auto_pattern, created_at)
             VALUES (?1, ?2, strftime('%s', 'now'))",
            rusqlite::params![prefix, auto_pattern],
        ).with_context(|| format!("Failed to insert concept '{prefix}'"))?;

        if rows_affected > 0 {
            created.push(prefix.clone());
        }
    }

    created.sort();
    Ok(created)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "concepts_tests.rs"]
mod tests;
