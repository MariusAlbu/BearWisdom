// =============================================================================
// search/history.rs  —  search history + saved searches  (Phase 6)
//
// Tracks recent search queries and lets users save/pin frequently-used ones.
// Backed by the `search_history` table in SQLite.
// =============================================================================

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHistoryEntry {
    pub id: i64,
    pub query: String,
    pub query_type: String,
    pub scope: Option<String>,
    pub is_saved: bool,
    pub last_used_at: i64,
    pub use_count: u32,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Record a search.  If an identical (query, query_type) row exists, its
/// `use_count` is incremented and `last_used_at` updated.  Otherwise a new
/// row is inserted.
pub fn record_search(
    conn: &Connection,
    query: &str,
    query_type: &str,
    scope: Option<&str>,
) -> Result<()> {
    let updated = conn
        .execute(
            "UPDATE search_history
             SET use_count    = use_count + 1,
                 last_used_at = strftime('%s', 'now')
             WHERE query = ?1 AND query_type = ?2",
            rusqlite::params![query, query_type],
        )
        .context("Failed to update search history")?;

    if updated == 0 {
        conn.execute(
            "INSERT INTO search_history (query, query_type, scope)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![query, query_type, scope],
        )
        .context("Failed to insert search history")?;
    }

    Ok(())
}

/// Return the most recently used searches, optionally filtered by type.
pub fn recent_searches(
    conn: &Connection,
    query_type: Option<&str>,
    limit: usize,
) -> Result<Vec<SearchHistoryEntry>> {
    let effective_limit = if limit == 0 { 50 } else { limit.min(200) };

    if let Some(qt) = query_type {
        let sql = format!(
            "SELECT id, query, query_type, scope, is_saved, last_used_at, use_count
             FROM search_history
             WHERE query_type = ?1
             ORDER BY last_used_at DESC
             LIMIT {effective_limit}"
        );
        query_entries(conn, &sql, rusqlite::params![qt])
    } else {
        let sql = format!(
            "SELECT id, query, query_type, scope, is_saved, last_used_at, use_count
             FROM search_history
             ORDER BY last_used_at DESC
             LIMIT {effective_limit}"
        );
        query_entries(conn, &sql, [])
    }
}

/// Return all pinned / saved searches.
pub fn saved_searches(conn: &Connection) -> Result<Vec<SearchHistoryEntry>> {
    query_entries(
        conn,
        "SELECT id, query, query_type, scope, is_saved, last_used_at, use_count
         FROM search_history
         WHERE is_saved = 1
         ORDER BY last_used_at DESC",
        [],
    )
}

/// Toggle the saved/pinned flag on an entry.  Returns the new `is_saved` state.
pub fn toggle_saved(conn: &Connection, id: i64) -> Result<bool> {
    let current: i32 = conn
        .query_row(
            "SELECT is_saved FROM search_history WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .context("Search history entry not found")?;

    let new_val = if current == 0 { 1 } else { 0 };
    conn.execute(
        "UPDATE search_history SET is_saved = ?1 WHERE id = ?2",
        rusqlite::params![new_val, id],
    )
    .context("Failed to toggle saved status")?;

    Ok(new_val == 1)
}

/// Delete the oldest non-saved entries, keeping at most `max_entries` unsaved
/// rows.  Returns the number of rows deleted.
pub fn prune_history(conn: &Connection, max_entries: usize) -> Result<u32> {
    let deleted = conn
        .execute(
            "DELETE FROM search_history
             WHERE is_saved = 0
               AND id NOT IN (
                   SELECT id FROM search_history
                   WHERE is_saved = 0
                   ORDER BY last_used_at DESC
                   LIMIT ?1
               )",
            [max_entries as i64],
        )
        .context("Failed to prune search history")?;

    Ok(deleted as u32)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn query_entries<P: rusqlite::Params>(
    conn: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<SearchHistoryEntry>> {
    let mut stmt = conn.prepare(sql).context("Failed to prepare history query")?;
    let rows = stmt
        .query_map(params, |row| {
            Ok(SearchHistoryEntry {
                id: row.get(0)?,
                query: row.get(1)?,
                query_type: row.get(2)?,
                scope: row.get(3)?,
                is_saved: row.get::<_, i32>(4)? != 0,
                last_used_at: row.get(5)?,
                use_count: row.get(6)?,
            })
        })
        .context("Failed to query search history")?;

    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect history results")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
