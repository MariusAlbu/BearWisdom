// =============================================================================
// bridge/graph_bridge.rs  —  merges LSP results into the SQLite graph
//
// GraphBridge takes resolved edges from an LSP server and writes them into
// the `edges` table with confidence=1.0 and records provenance in
// `lsp_edge_meta`.
//
// When a file changes, the caller must invalidate the corresponding
// lsp_edge_meta rows so they get re-resolved on the next LSP pass.
// =============================================================================

use crate::db::Database;
use crate::lsp::manager::LspManager;
use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Merges LSP-resolved edges into the index graph.
///
/// Each LSP-resolved edge:
///   1. Is upserted into `edges` with confidence = 1.0.
///   2. Gets a row in `lsp_edge_meta` recording the server name and timestamp.
///
/// When a file changes, the caller must invalidate the corresponding
/// lsp_edge_meta rows so they get re-resolved on the next LSP pass.
pub struct GraphBridge {
    db: Arc<Mutex<Database>>,
    lsp: Arc<LspManager>,
    workspace_root: PathBuf,
    /// Don't invoke LSP if tree-sitter confidence is already >= this value.
    pub confidence_threshold: f64,
}

impl GraphBridge {
    pub fn new(
        db: Arc<Mutex<Database>>,
        lsp: Arc<LspManager>,
        workspace_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            db,
            lsp,
            workspace_root: workspace_root.into(),
            confidence_threshold: 0.95,
        }
    }

    // -----------------------------------------------------------------------
    // URI / path helpers
    // -----------------------------------------------------------------------

    /// Convert a `file:///` URI to a path relative to `self.workspace_root`.
    ///
    /// Works purely in forward-slash string space so that it is host-platform
    /// independent and test-friendly regardless of what path style is used for
    /// `workspace_root`.
    ///
    /// On Windows, `file:///C:/foo/bar` → without-scheme = `C:/foo/bar`.
    /// On POSIX,   `file:///home/user`  → without-scheme = `home/user` (leading
    /// slash was consumed by the `file:///` prefix); we prepend `/` to restore.
    ///
    /// Returns `None` if the URI does not start with `file:///`.
    pub fn uri_to_relative_path(&self, uri: &str) -> Option<String> {
        let without_scheme = uri.strip_prefix("file:///")?;

        // Reconstruct the normalised absolute path string (forward slashes).
        let abs_str: String = {
            // Detect a Windows drive letter at the start (e.g. "C:/...").
            let has_drive = without_scheme
                .chars()
                .next()
                .map(|c| c.is_ascii_alphabetic())
                .unwrap_or(false)
                && without_scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && without_scheme.len() >= 2
                && without_scheme.as_bytes()[1] == b':';

            if has_drive {
                without_scheme.replace('\\', "/")
            } else {
                // POSIX — restore the leading slash.
                format!("/{without_scheme}")
            }
        };

        // Normalise the workspace root to forward slashes for comparison.
        let root_str = self
            .workspace_root
            .to_string_lossy()
            .replace('\\', "/");

        // Strip the workspace root prefix and the separating `/`.
        let root_prefix = if root_str.ends_with('/') {
            root_str.clone()
        } else {
            format!("{root_str}/")
        };

        let rel = abs_str.strip_prefix(&root_prefix)?.to_string();
        Some(rel)
    }

    // -----------------------------------------------------------------------
    // Symbol lookup
    // -----------------------------------------------------------------------

    /// Find the narrowest symbol in the DB that contains `(line, col)` in
    /// the given file URI.  Returns the symbol's `id`, or `None`.
    pub fn location_to_symbol_id(&self, uri: &str, line: u32, _col: u32) -> Result<Option<i64>> {
        let file_path = match self.uri_to_relative_path(uri) {
            Some(p) => p,
            None => return Ok(None),
        };

        let guard = self.db.lock().unwrap();
        let id: Option<i64> = guard
            .conn
            .query_row(
                "SELECT s.id
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE f.path = ?1
                   AND s.line <= ?2
                   AND COALESCE(s.end_line, s.line) >= ?2
                 ORDER BY (COALESCE(s.end_line, s.line) - s.line) ASC
                 LIMIT 1",
                rusqlite::params![file_path, line],
                |row| row.get(0),
            )
            .optional()
            .context("location_to_symbol_id query")?;

        Ok(id)
    }

    // -----------------------------------------------------------------------
    // Edge persistence
    // -----------------------------------------------------------------------

    /// Upsert an LSP-resolved edge and record provenance in `lsp_edge_meta`.
    ///
    /// Returns `true` if a new edge was written or an existing edge was
    /// upgraded; `false` if it already existed at confidence 1.0.
    pub fn persist_lsp_edge(
        &self,
        source_id: i64,
        target_id: i64,
        kind: &str,
        source_line: Option<u32>,
        server: &str,
    ) -> Result<bool> {
        let guard = self.db.lock().unwrap();

        // Try to upgrade an existing edge that is not yet at 1.0.
        let updated = guard
            .conn
            .execute(
                "UPDATE edges SET confidence = 1.0
                 WHERE source_id = ?1 AND target_id = ?2 AND kind = ?3
                   AND source_line IS ?4
                   AND confidence < 1.0",
                rusqlite::params![source_id, target_id, kind, source_line],
            )
            .context("persist_lsp_edge UPDATE")?;

        if updated == 0 {
            // No existing sub-1.0 edge — try inserting (may be a no-op if already at 1.0).
            guard
                .conn
                .execute(
                    "INSERT OR IGNORE INTO edges (source_id, target_id, kind, source_line, confidence)
                     VALUES (?1, ?2, ?3, ?4, 1.0)",
                    rusqlite::params![source_id, target_id, kind, source_line],
                )
                .context("persist_lsp_edge INSERT")?;
        }

        // Retrieve the edge's rowid so we can write lsp_edge_meta.
        let rowid: Option<i64> = guard
            .conn
            .query_row(
                "SELECT rowid FROM edges
                 WHERE source_id = ?1 AND target_id = ?2 AND kind = ?3
                   AND source_line IS ?4",
                rusqlite::params![source_id, target_id, kind, source_line],
                |row| row.get(0),
            )
            .optional()
            .context("persist_lsp_edge SELECT rowid")?;

        let Some(edge_rowid) = rowid else {
            return Ok(false);
        };

        // Check whether lsp_edge_meta already has a fresh entry for this edge.
        let meta_exists: bool = guard
            .conn
            .query_row(
                "SELECT 1 FROM lsp_edge_meta WHERE edge_rowid = ?1",
                [edge_rowid],
                |_| Ok(true),
            )
            .optional()
            .context("persist_lsp_edge SELECT meta")?
            .unwrap_or(false);

        // Upsert lsp_edge_meta.
        guard
            .conn
            .execute(
                "INSERT OR REPLACE INTO lsp_edge_meta (edge_rowid, source, server, resolved_at)
                 VALUES (?1, 'lsp', ?2, strftime('%s', 'now'))",
                rusqlite::params![edge_rowid, server],
            )
            .context("persist_lsp_edge UPSERT meta")?;

        // Return true if something actually changed (new edge or new meta).
        Ok(updated > 0 || !meta_exists)
    }

    /// Upgrade an existing edge's confidence to `new_confidence` if it is
    /// currently lower.  Returns `true` if a row was updated.
    pub fn upgrade_confidence(
        &self,
        source_id: i64,
        target_id: i64,
        kind: &str,
        new_confidence: f64,
    ) -> Result<bool> {
        let guard = self.db.lock().unwrap();
        let rows = guard
            .conn
            .execute(
                "UPDATE edges
                 SET confidence = MAX(confidence, ?1)
                 WHERE source_id = ?2 AND target_id = ?3 AND kind = ?4
                   AND confidence < ?1",
                rusqlite::params![new_confidence, source_id, target_id, kind],
            )
            .context("upgrade_confidence UPDATE")?;

        Ok(rows > 0)
    }

    /// Delete LSP provenance for all edges whose source or target belongs to
    /// symbols in `file_path`, and reset those edges' confidence to 0.50.
    ///
    /// Returns the number of edges invalidated.
    pub fn invalidate_file_edges(&self, file_path: &str) -> Result<u32> {
        let guard = self.db.lock().unwrap();

        // 1. Collect edge rowids that have LSP meta AND belong to the file.
        let mut stmt = guard.conn.prepare(
            "SELECT lm.edge_rowid
             FROM lsp_edge_meta lm
             JOIN edges e ON e.rowid = lm.edge_rowid
             WHERE e.source_id IN (
                 SELECT id FROM symbols
                 WHERE file_id = (SELECT id FROM files WHERE path = ?1)
             )
             OR e.target_id IN (
                 SELECT id FROM symbols
                 WHERE file_id = (SELECT id FROM files WHERE path = ?1)
             )",
        )?;

        let rowids: Vec<i64> = stmt
            .query_map([file_path], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        if rowids.is_empty() {
            return Ok(0);
        }

        let count = rowids.len() as u32;

        // 2. Delete the meta rows.
        for &rid in &rowids {
            guard
                .conn
                .execute("DELETE FROM lsp_edge_meta WHERE edge_rowid = ?1", [rid])
                .context("invalidate_file_edges DELETE meta")?;
        }

        // 3. Reset confidence to 0.50 for those edges.
        for &rid in &rowids {
            guard
                .conn
                .execute(
                    "UPDATE edges SET confidence = 0.50 WHERE rowid = ?1",
                    [rid],
                )
                .context("invalidate_file_edges UPDATE confidence")?;
        }

        Ok(count)
    }

    // -----------------------------------------------------------------------
    // Async LSP resolution
    // -----------------------------------------------------------------------

    /// Ask the LSP server for the definition at `(file_path, line, col)`.
    ///
    /// Returns `Some((target_symbol_id, 1.0))` if the location maps to a
    /// known symbol, or `None` if the server found nothing or the location
    /// does not resolve to a tracked symbol.
    pub async fn resolve_definition_via_lsp(
        &self,
        file_path: &str,
        line: u32,
        col: u32,
    ) -> Result<Option<(i64, f64)>> {
        let locations = self.lsp.goto_definition(file_path, line, col).await?;

        for loc in locations {
            if let Some(sym_id) =
                self.location_to_symbol_id(&loc.uri, loc.range.start.line, loc.range.start.character)?
            {
                return Ok(Some((sym_id, 1.0)));
            }
        }

        Ok(None)
    }

    /// Ask the LSP server for all references at `(file_path, line, col)`.
    ///
    /// Returns a vec of `(symbol_id, 1.0)` for every location that maps to a
    /// tracked symbol.
    pub async fn resolve_references_via_lsp(
        &self,
        file_path: &str,
        line: u32,
        col: u32,
    ) -> Result<Vec<(i64, f64)>> {
        let locations = self.lsp.find_references(file_path, line, col).await?;

        let mut results = Vec::new();
        for loc in locations {
            if let Some(sym_id) =
                self.location_to_symbol_id(&loc.uri, loc.range.start.line, loc.range.start.character)?
            {
                results.push((sym_id, 1.0));
            }
        }

        Ok(results)
    }

    // -----------------------------------------------------------------------
    // Reference-site column helper
    // -----------------------------------------------------------------------

    /// Find the byte-offset column of the first occurrence of `target_name`
    /// on the given (0-based) `line` of `file_path`.
    ///
    /// `file_path` is relative to `workspace_root`.  Returns `0` if the file
    /// cannot be read, the line does not exist, or the name is not found on
    /// that line.
    pub fn find_target_column(
        workspace_root: &std::path::Path,
        file_path: &str,
        line: u32,
        target_name: &str,
    ) -> u32 {
        let full_path = workspace_root.join(file_path);
        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => return 0,
        };
        let line_content = match content.lines().nth(line as usize) {
            Some(l) => l,
            None => return 0,
        };
        line_content
            .find(target_name)
            .map(|offset| offset as u32)
            .unwrap_or(0)
    }

    // -----------------------------------------------------------------------
    // Internal accessor (used by BackgroundEnricher in this module)
    // -----------------------------------------------------------------------

    /// Return a reference to the shared database — used by `BackgroundEnricher`.
    pub(crate) fn db(&self) -> &Arc<Mutex<Database>> {
        &self.db
    }

    /// Return a reference to the LSP manager — used by `BackgroundEnricher`.
    pub(crate) fn lsp(&self) -> &Arc<LspManager> {
        &self.lsp
    }

    /// Return the workspace root path — used by `BackgroundEnricher`.
    pub(crate) fn workspace_root(&self) -> &std::path::Path {
        &self.workspace_root
    }

    // -----------------------------------------------------------------------
    // Stats
    // -----------------------------------------------------------------------

    pub fn unresolved_ref_count(&self) -> Result<u32> {
        let guard = self.db.lock().unwrap();
        let count: i64 = guard
            .conn
            .query_row("SELECT COUNT(*) FROM unresolved_refs", [], |row| row.get(0))
            .context("unresolved_ref_count")?;
        Ok(count as u32)
    }

    pub fn lsp_edge_count(&self) -> Result<u32> {
        let guard = self.db.lock().unwrap();
        let count: i64 = guard
            .conn
            .query_row("SELECT COUNT(*) FROM lsp_edge_meta", [], |row| row.get(0))
            .context("lsp_edge_count")?;
        Ok(count as u32)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "graph_bridge_tests.rs"]
mod tests;
