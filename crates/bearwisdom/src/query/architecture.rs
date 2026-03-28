// =============================================================================
// query/architecture.rs  —  project-level overview query
//
// Returns a single `ArchitectureOverview` struct that answers "what is this
// codebase made of?" without needing to walk the graph manually.
//
// The three sub-queries are:
//   1. Totals   — COUNT rows from files, symbols, edges.
//   2. Language breakdown — per-language file + symbol counts.
//   3. Hotspots  — symbols with the most incoming edges (the "most depended-on"
//                  pieces of the codebase).
//   4. Entry points — public classes and functions (low in-degree, high
//                     out-degree — top-level API surface).
// =============================================================================

use crate::db::Database;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// A summary of how many files and symbols belong to a single language.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageStats {
    /// Language string as stored in the `files` table, e.g. "csharp", "typescript".
    pub language: String,
    /// Number of indexed files in this language.
    pub file_count: u32,
    /// Number of symbols extracted from those files.
    pub symbol_count: u32,
}

/// A symbol that is referenced by many others — the "hotspots" of the codebase.
/// High incoming-edge count means many callers / dependents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotspotSymbol {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file_path: String,
    /// Number of edges whose target is this symbol.
    pub incoming_refs: u32,
}

/// A lightweight summary of a single symbol, used for entry-point lists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolSummary {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file_path: String,
    /// 1-based line number where the symbol is defined.
    pub line: u32,
}

/// The full architecture overview returned by [`get_overview`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureOverview {
    /// Total number of indexed files.
    pub total_files: u32,
    /// Total number of extracted symbols.
    pub total_symbols: u32,
    /// Total number of resolved edges.
    pub total_edges: u32,
    /// Per-language breakdown, sorted by file count descending.
    pub languages: Vec<LanguageStats>,
    /// Top 20 symbols by incoming reference count.
    pub hotspots: Vec<HotspotSymbol>,
    /// Public classes and top-level functions (the API surface).
    pub entry_points: Vec<SymbolSummary>,
}

// ---------------------------------------------------------------------------
// Public function
// ---------------------------------------------------------------------------

/// Build and return a complete `ArchitectureOverview` for the database.
///
/// All four sub-queries run against the open database; no indexing happens here.
/// Build overview with default limits (10 hotspots, 20 entry points).
pub fn get_overview(db: &Database) -> Result<ArchitectureOverview> {
    get_overview_with_limits(db, 10, 20)
}

/// Build overview with custom limits.
pub fn get_overview_with_limits(
    db: &Database,
    hotspot_limit: usize,
    entry_point_limit: usize,
) -> Result<ArchitectureOverview> {
    let conn = &db.conn;

    // --- 1. Totals ---
    let total_files: u32 =
        conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .context("Failed to count files")?;

    let total_symbols: u32 =
        conn.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
            .context("Failed to count symbols")?;

    let total_edges: u32 =
        conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .context("Failed to count edges")?;

    // --- 2. Language breakdown ---
    // LEFT JOIN so languages with zero symbols still appear.
    let languages = {
        let mut stmt = conn.prepare(
            "SELECT f.language,
                    COUNT(DISTINCT f.id)  AS file_count,
                    COUNT(s.id)           AS symbol_count
             FROM files f
             LEFT JOIN symbols s ON s.file_id = f.id
             GROUP BY f.language
             ORDER BY file_count DESC",
        ).context("Failed to prepare language stats query")?;

        let rows = stmt.query_map([], |row| {
            Ok(LanguageStats {
                language:     row.get(0)?,
                file_count:   row.get(1)?,
                symbol_count: row.get(2)?,
            })
        }).context("Failed to execute language stats query")?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect language stats")?
    };

    // --- 3. Hotspots (symbols with most incoming edges) ---
    let hotspots = {
        let mut stmt = conn.prepare(
            "SELECT s.name,
                    s.qualified_name,
                    s.kind,
                    f.path,
                    COUNT(e.source_id) AS incoming_refs
             FROM symbols s
             JOIN files f   ON f.id = s.file_id
             JOIN edges e   ON e.target_id = s.id
             GROUP BY s.id
             ORDER BY incoming_refs DESC
             LIMIT ?1",
        ).context("Failed to prepare hotspots query")?;

        let rows = stmt.query_map([hotspot_limit as i64], |row| {
            Ok(HotspotSymbol {
                name:          row.get(0)?,
                qualified_name: row.get(1)?,
                kind:          row.get(2)?,
                file_path:     row.get(3)?,
                incoming_refs: row.get(4)?,
            })
        }).context("Failed to execute hotspots query")?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect hotspots")?
    };

    // --- 4. Entry points (public classes + functions, limited to 50) ---
    // We define "entry point" as a public symbol whose kind is class or function,
    // making them the likely API surface.
    let entry_points = {
        let mut stmt = conn.prepare(
            "SELECT s.name, s.qualified_name, s.kind, f.path, s.line
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE s.visibility = 'public'
               AND s.kind IN ('class', 'interface', 'function', 'struct')
             ORDER BY f.path, s.line
             LIMIT ?1",
        ).context("Failed to prepare entry points query")?;

        let rows = stmt.query_map([entry_point_limit as i64], |row| {
            Ok(SymbolSummary {
                name:          row.get(0)?,
                qualified_name: row.get(1)?,
                kind:          row.get(2)?,
                file_path:     row.get(3)?,
                line:          row.get(4)?,
            })
        }).context("Failed to execute entry points query")?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect entry points")?
    };

    Ok(ArchitectureOverview {
        total_files,
        total_symbols,
        total_edges,
        languages,
        hotspots,
        entry_points,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "architecture_tests.rs"]
mod tests;
