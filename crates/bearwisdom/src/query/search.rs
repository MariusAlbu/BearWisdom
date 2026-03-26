// =============================================================================
// query/search.rs  —  FTS5 full-text symbol search
//
// Uses the `symbols_fts` FTS5 virtual table (created in db/schema.rs) to do
// BM25-ranked full-text search across:
//   • symbol names       (e.g. "GetById", "CatalogService")
//   • qualified names    (e.g. "Catalog.CatalogService.GetById")
//   • signatures         (e.g. "Task<CatalogItem> GetById(int id)")
//   • doc comments       (e.g. "Returns the catalog item with the given ID")
//
// FTS5 'rank' column:
//   SQLite FTS5 returns a negative rank (lower = better match).  We negate it
//   before returning so callers see positive scores with higher = better.
//
// Query syntax (passed straight to FTS5):
//   • Simple word:  "catalog" — matches any of the four indexed columns.
//   • Prefix:       "catalog*" — prefix match.
//   • Phrase:       '"get catalog"' — exact phrase.
//   • Column scope: "name:GetById" — match only the name column.
//   See https://www.sqlite.org/fts5.html#full_text_query_syntax for full syntax.
//
// Fallback:
//   If the FTS5 query returns no results (e.g. if symbols_fts is empty because
//   the database predates the FTS triggers), we also attempt a LIKE-based fuzzy
//   fallback search on `symbols.name` and `symbols.qualified_name`.
// =============================================================================

use crate::db::Database;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

/// One search result from the FTS5 index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub name: String,
    pub qualified_name: String,
    /// Symbol kind string, e.g. "class", "method".
    pub kind: String,
    pub file_path: String,
    /// 1-based line number of the symbol definition.
    pub start_line: u32,
    pub signature: Option<String>,
    /// BM25 relevance score — higher is a better match.
    /// FTS5 returns negative rank; we negate it here for a natural ordering.
    pub score: f64,
}

// ---------------------------------------------------------------------------
// Public function
// ---------------------------------------------------------------------------

/// Full-text search across symbol names, qualified names, signatures, and doc
/// comments.
///
/// `query`  — FTS5 query string (plain words, prefix with `*`, phrases in `""`).
/// `limit`  — maximum results to return (pass 0 for no limit, capped at 500).
///
/// Results are returned in descending relevance order (highest score first).
pub fn search_symbols(db: &Database, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let conn = &db.conn;

    // Guard: FTS5 needs at least one term.
    if query.trim().is_empty() {
        return Ok(vec![]);
    }

    // Cap the limit — an unbounded FTS query on a large index is expensive.
    let effective_limit = if limit == 0 { 500 } else { limit.min(500) };

    // --- Primary: FTS5 query ---
    // `rank` in FTS5 is a negative BM25 score; ORDER BY rank ascending puts
    // the best matches first.  We negate it in the SELECT list so callers see
    // positive values.
    let fts_sql = format!(
        "SELECT s.name,
                s.qualified_name,
                s.kind,
                f.path       AS file_path,
                s.line       AS start_line,
                s.signature,
                (-fts.rank)  AS score
         FROM symbols_fts fts
         JOIN symbols s ON s.id = fts.rowid
         JOIN files   f ON f.id = s.file_id
         WHERE symbols_fts MATCH ?1
         ORDER BY fts.rank
         LIMIT {effective_limit}"
    );

    let mut stmt = conn.prepare(&fts_sql)
        .context("Failed to prepare FTS5 search query")?;

    let rows = stmt.query_map([query], |row| {
        Ok(SearchResult {
            name:          row.get(0)?,
            qualified_name: row.get(1)?,
            kind:          row.get(2)?,
            file_path:     row.get(3)?,
            start_line:    row.get(4)?,
            signature:     row.get(5)?,
            score:         row.get(6)?,
        })
    }).context("Failed to execute FTS5 search query")?;

    let results: rusqlite::Result<Vec<SearchResult>> = rows.collect();

    match results {
        Ok(ref r) if !r.is_empty() => return Ok(results.unwrap()),
        Err(e) => {
            // If FTS5 returns an error (e.g. invalid query syntax), fall through
            // to the LIKE fallback rather than hard-failing.
            tracing::debug!("FTS5 search error, falling back to LIKE: {e}");
        }
        Ok(_) => {
            // Zero FTS5 results — still try the LIKE fallback.
        }
    }

    // --- Fallback: LIKE search on name and qualified_name ---
    // Useful when symbols_fts is empty (pre-trigger data) or when the FTS
    // query string is not a valid FTS5 expression.
    let like_pattern = format!("%{query}%");
    let like_sql = format!(
        "SELECT s.name,
                s.qualified_name,
                s.kind,
                f.path AS file_path,
                s.line AS start_line,
                s.signature,
                0.0    AS score
         FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE s.name           LIKE ?1 ESCAPE '\\'
            OR s.qualified_name LIKE ?1 ESCAPE '\\'
         ORDER BY s.qualified_name
         LIMIT {effective_limit}"
    );

    let mut stmt = conn.prepare(&like_sql)
        .context("Failed to prepare LIKE fallback query")?;

    let rows = stmt.query_map([&like_pattern], |row| {
        Ok(SearchResult {
            name:          row.get(0)?,
            qualified_name: row.get(1)?,
            kind:          row.get(2)?,
            file_path:     row.get(3)?,
            start_line:    row.get(4)?,
            signature:     row.get(5)?,
            score:         row.get(6)?,
        })
    }).context("Failed to execute LIKE fallback query")?;

    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect LIKE fallback results")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
