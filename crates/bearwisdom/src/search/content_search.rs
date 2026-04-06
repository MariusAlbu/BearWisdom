// =============================================================================
// search/content_search.rs  —  FTS5 trigram content query layer
//
// Two search modes:
//
//   1. `search_content` — file-level: asks the FTS5 index "which files contain
//      this string?" and returns ranked file records.  Fast; no disk I/O after
//      the index is warm.
//
//   2. `search_content_with_lines` — line-level: uses FTS5 to get the
//      candidate file set, then runs `grep_search` on those files only to
//      produce exact line/column results.
//
// Design notes:
//   • Trigram FTS5 requires at least 3 characters; shorter queries get an
//     empty result rather than a full-scan fallback (which would be slow and
//     semantically wrong for an IDE).
//   • FTS5 rank is a negative float (more negative = better match).  We
//     negate it to produce a positive ascending score for callers.
//   • Scope filtering runs in Rust after the SQL query because FTS5 does not
//     support joining with a WHERE clause that touches non-FTS columns without
//     losing the rank ordering.  The extra rows filtered out are a small
//     fraction of the total result set.
//   • `search_content_with_lines` builds a temporary directory-scoped
//     SearchScope that points grep_search at only the matched files, using
//     an include_glob list.  This avoids re-walking the whole tree.
// =============================================================================

use std::path::Path;
use std::sync::{atomic::AtomicBool, Arc};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::db::Database;
use crate::search::grep::{grep_search, GrepMatch, GrepOptions};
use crate::search::scope::SearchScope;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A file-level result from the FTS5 trigram content index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentSearchResult {
    /// Database row id from the `files` table.
    pub file_id: i64,
    /// Relative path (forward-slash).
    pub file_path: String,
    /// Language tag from the `files` table.
    pub language: String,
    /// Relevance score — higher is more relevant.  Derived from FTS5 rank.
    pub score: f64,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Return which files in the index contain `query`.
///
/// Uses the FTS5 trigram index for fast substring search.  Returns an empty
/// vec for queries shorter than 3 bytes (trigram minimum).
///
/// Results are ordered by FTS5 rank (best match first) and filtered by
/// `scope` after retrieval.
pub fn search_content(
    db: &Database,
    query: &str,
    scope: &SearchScope,
    limit: usize,
) -> Result<Vec<ContentSearchResult>> {
    if query.len() < 3 {
        return Ok(vec![]);
    }

    // FTS5 MATCH syntax for a literal substring is a quoted phrase.
    let fts_query = quote_fts_query(query);
    let effective_limit = limit.max(1) as i64;

    let sql = "
        SELECT f.id, f.path, f.language, (-fts.rank) AS score
        FROM fts_content fts
        JOIN files f ON f.id = fts.rowid
        WHERE fts_content MATCH ?1
        ORDER BY fts.rank
        LIMIT ?2
    ";

    let mut stmt = db
        .prepare(sql)
        .context("Failed to prepare content search query")?;

    let rows = stmt
        .query_map(rusqlite::params![fts_query, effective_limit], |row| {
            Ok(ContentSearchResult {
                file_id: row.get(0)?,
                file_path: row.get(1)?,
                language: row.get(2)?,
                score: row.get(3)?,
            })
        })
        .context("Failed to execute content search query")?;

    let mut results: Vec<ContentSearchResult> = rows
        .filter_map(|r| match r {
            Ok(row) => Some(row),
            Err(e) => {
                tracing::warn!("content_search row error: {e}");
                None
            }
        })
        .filter(|r| scope.matches_file(&r.file_path, &r.language))
        .collect();

    // Re-apply limit after scope filtering (SQL LIMIT was a pre-filter upper bound).
    results.truncate(limit);

    debug!(count = results.len(), query, "search_content complete");
    Ok(results)
}

/// Return line-level matches for `query` by combining FTS5 candidate
/// selection with a precise grep pass over the matched files.
///
/// Steps:
///   1. Run FTS5 to get the set of files likely to contain `query`.
///   2. Run `grep_search` restricted to those files only.
///
/// Returns an empty vec for queries shorter than 3 bytes.
pub fn search_content_with_lines(
    db: &Database,
    project_root: &Path,
    query: &str,
    scope: &SearchScope,
    limit: usize,
) -> Result<Vec<GrepMatch>> {
    if query.len() < 3 {
        return Ok(vec![]);
    }

    // Step 1: get candidate files from FTS5.
    // We fetch more candidates than `limit` to give grep room to find
    // matches after applying the scope filter.
    let candidate_limit = (limit * 4).max(100);
    let candidates = search_content(db, query, scope, candidate_limit)?;

    if candidates.is_empty() {
        return Ok(vec![]);
    }

    // Step 2: build a scope that restricts grep to just those file paths.
    let mut restricted_scope = scope.clone();
    for candidate in &candidates {
        restricted_scope
            .include_globs
            .push(candidate.file_path.clone());
    }

    let opts = GrepOptions {
        case_sensitive: true,
        whole_word: false,
        regex: false,
        max_results: limit,
        scope: restricted_scope,
        context_lines: 0,
    };

    let cancelled = Arc::new(AtomicBool::new(false));
    let matches = grep_search(project_root, query, &opts, &cancelled)
        .context("grep pass in search_content_with_lines failed")?;

    debug!(
        count = matches.len(),
        query,
        files = candidates.len(),
        "search_content_with_lines complete"
    );
    Ok(matches)
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Build an FTS5 MATCH expression for `query`.
///
/// Single-word queries (no whitespace) are phrase-quoted directly:
///   `budget` → `"budget"`
///
/// Multi-word queries are split on whitespace and each token is individually
/// phrase-quoted, then joined with OR (IDE-040 fix).  FTS5 phrase matching
/// requires the exact string to appear contiguously, so `"budget service"`
/// would only match files containing that exact phrase.  Splitting to
/// `"budget" OR "service"` matches files containing either word, which is
/// the correct behaviour for an IDE search bar.
///
/// Tokens shorter than 3 characters are skipped (FTS5 trigram minimum).
/// If all tokens are filtered out the caller will receive an empty result.
///
/// Each token has embedded double-quote characters doubled to satisfy FTS5
/// syntax rules.  Example: `O'Brien "test"` → `"O'Brien" OR """test"""`
fn quote_fts_query(query: &str) -> String {
    let tokens: Vec<String> = query
        .split_whitespace()
        .filter(|t| t.len() >= 3)
        .map(|t| {
            let escaped = t.replace('"', "\"\"");
            format!("\"{escaped}\"")
        })
        .collect();

    if tokens.is_empty() {
        // Caller should have already checked length; return something that
        // produces zero rows rather than a syntax error.
        String::from("\"\"")
    } else {
        tokens.join(" OR ")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "content_search_tests.rs"]
mod tests;
