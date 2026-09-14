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
use crate::query::QueryResult;
use anyhow::Context;
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

/// Build a forgiving fallback for the way coding agents naturally phrase
/// symbol searches. FTS5 treats whitespace-separated terms as AND, which is
/// useful for deliberate FTS expressions but surprising for queries that list
/// several candidate identifiers. When the exact query has no hits, retry the
/// distinct plain terms as OR alternatives.
fn fallback_or_query(query: &str) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    let terms: Vec<String> = query
        .split_whitespace()
        .filter_map(|raw| {
            let term = raw
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != ':' && c != '-');
            if term.len() < 2
                || matches!(term.to_ascii_uppercase().as_str(), "AND" | "OR" | "NOT")
                || !seen.insert(term.to_ascii_lowercase())
            {
                return None;
            }
            Some(format!("\"{}\"", term.replace('"', "\"\"")))
        })
        .collect();

    (terms.len() > 1).then(|| terms.join(" OR "))
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
pub fn search_symbols(
    db: &Database,
    query: &str,
    limit: usize,
    opts: &super::QueryOptions,
) -> QueryResult<Vec<SearchResult>> {
    let _timer = db.timer("search_symbols");
    let conn = db.conn();

    // Guard: FTS5 needs at least one term.
    if query.trim().is_empty() {
        return Ok(vec![]);
    }

    // Cap the limit — an unbounded FTS query on a large index is expensive.
    let effective_limit = if limit == 0 { 500 } else { limit.min(500) };

    let sig_col = if opts.include_signature {
        "s.signature"
    } else {
        "NULL"
    };

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
                {sig_col}    AS signature,
                (-fts.rank)  AS score
         FROM symbols_fts fts
         JOIN symbols s ON s.id = fts.rowid
         JOIN files   f ON f.id = s.file_id
         WHERE symbols_fts MATCH ?1
           AND s.origin = 'internal'
         ORDER BY fts.rank
         LIMIT {effective_limit}"
    );

    let mut stmt = conn
        .prepare(&fts_sql)
        .context("Failed to prepare FTS5 search query")?;

    let mut run_fts = |fts_query: &str| -> anyhow::Result<Vec<SearchResult>> {
        let rows = stmt.query_map([fts_query], |row| {
            Ok(SearchResult {
                name: row.get(0)?,
                qualified_name: row.get(1)?,
                kind: row.get(2)?,
                file_path: row.get(3)?,
                start_line: row.get(4)?,
                signature: row.get(5)?,
                score: row.get(6)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect FTS5 search results")
    };

    match run_fts(query) {
        Ok(results) if !results.is_empty() => return Ok(results),
        Ok(_) => {}
        Err(e) => tracing::debug!("FTS5 search error: {e}"),
    }

    if let Some(or_query) = fallback_or_query(query) {
        match run_fts(&or_query) {
            Ok(results) if !results.is_empty() => return Ok(results),
            Ok(_) => {}
            Err(e) => tracing::debug!("FTS5 OR fallback error: {e}"),
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
                {sig_col} AS signature,
                0.0    AS score
         FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE s.origin = 'internal'
           AND (s.name           LIKE ?1 ESCAPE '\\'
             OR s.qualified_name LIKE ?1 ESCAPE '\\')
         ORDER BY s.qualified_name
         LIMIT {effective_limit}"
    );

    let mut stmt = conn
        .prepare(&like_sql)
        .context("Failed to prepare LIKE fallback query")?;

    let rows = stmt
        .query_map([&like_pattern], |row| {
            Ok(SearchResult {
                name: row.get(0)?,
                qualified_name: row.get(1)?,
                kind: row.get(2)?,
                file_path: row.get(3)?,
                start_line: row.get(4)?,
                signature: row.get(5)?,
                score: row.get(6)?,
            })
        })
        .context("Failed to execute LIKE fallback query")?;

    Ok(rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect LIKE fallback results")?)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
