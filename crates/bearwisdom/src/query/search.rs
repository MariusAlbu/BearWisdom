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
mod tests {
    use super::*;
    use crate::db::Database;

    /// Insert a symbol and let the triggers populate symbols_fts.
    fn insert_symbol(
        db: &Database,
        path: &str,
        name: &str,
        qname: &str,
        kind: &str,
        sig: Option<&str>,
        doc: Option<&str>,
    ) -> i64 {
        let conn = &db.conn;
        conn.execute(
            "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'csharp', 0)
             ON CONFLICT(path) DO NOTHING",
            [path],
        ).unwrap();
        let fid: i64 = conn.query_row("SELECT id FROM files WHERE path=?1", [path], |r| r.get(0)).unwrap();

        conn.execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, signature, doc_comment)
             VALUES (?1, ?2, ?3, ?4, 1, 0, ?5, ?6)",
            rusqlite::params![fid, name, qname, kind, sig, doc],
        ).unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn search_finds_symbol_by_name() {
        let db = Database::open_in_memory().unwrap();
        insert_symbol(&db, "a.cs", "CatalogService", "App.CatalogService", "class", None, None);

        let results = search_symbols(&db, "CatalogService", 10).unwrap();
        assert!(!results.is_empty(), "Should find CatalogService");
        assert_eq!(results[0].name, "CatalogService");
    }

    #[test]
    fn search_prefix_match() {
        let db = Database::open_in_memory().unwrap();
        insert_symbol(&db, "a.cs", "CatalogService", "App.CatalogService", "class", None, None);
        insert_symbol(&db, "b.cs", "CatalogItem",    "App.CatalogItem",    "class", None, None);
        insert_symbol(&db, "c.cs", "OrderService",   "App.OrderService",   "class", None, None);

        // Prefix query: "Catalog*" should match CatalogService and CatalogItem.
        let results = search_symbols(&db, "Catalog*", 10).unwrap();
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"CatalogService"), "Should match CatalogService");
        assert!(names.contains(&"CatalogItem"),    "Should match CatalogItem");
        assert!(!names.contains(&"OrderService"),  "Should not match OrderService");
    }

    #[test]
    fn search_matches_in_doc_comment() {
        let db = Database::open_in_memory().unwrap();
        insert_symbol(
            &db, "a.cs", "GetItems", "App.GetItems", "method",
            None, Some("Returns all items from the authentication store"),
        );

        let results = search_symbols(&db, "authentication", 10).unwrap();
        assert!(!results.is_empty(), "Should find symbol via doc comment");
        assert_eq!(results[0].name, "GetItems");
    }

    #[test]
    fn search_returns_empty_for_nonexistent_term() {
        let db = Database::open_in_memory().unwrap();
        insert_symbol(&db, "a.cs", "FooService", "App.FooService", "class", None, None);

        let results = search_symbols(&db, "ZzzNotFoundXxx", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_respects_limit() {
        let db = Database::open_in_memory().unwrap();
        for i in 0..10 {
            insert_symbol(
                &db, "a.cs",
                &format!("Widget{i}"), &format!("App.Widget{i}"),
                "class", None, None,
            );
        }

        let results = search_symbols(&db, "Widget*", 3).unwrap();
        assert!(results.len() <= 3, "Should respect limit of 3");
    }

    #[test]
    fn search_empty_query_returns_empty() {
        let db = Database::open_in_memory().unwrap();
        let results = search_symbols(&db, "", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn search_matches_in_signature() {
        let db = Database::open_in_memory().unwrap();
        insert_symbol(
            &db, "a.cs", "Fetch", "App.Fetch", "method",
            Some("Task<CatalogItem> Fetch(int id)"), None,
        );

        let results = search_symbols(&db, "CatalogItem", 10).unwrap();
        // The FTS index includes the signature, so "CatalogItem" in the sig should match.
        assert!(!results.is_empty(), "Should match via signature");
    }
}
