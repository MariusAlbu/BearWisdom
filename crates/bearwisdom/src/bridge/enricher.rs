// =============================================================================
// bridge/enricher.rs  —  background enrichment of unresolved refs via LSP
//
// BackgroundEnricher runs during idle time and drains the `unresolved_refs`
// table by asking the LSP server to resolve each reference.  Resolved edges
// are written back via GraphBridge.
// =============================================================================

use crate::bridge::graph_bridge::GraphBridge;
use crate::db::Database;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Progress snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnrichmentProgress {
    /// Total unresolved refs at the start of this pass.
    pub total_unresolved: u32,
    /// Number successfully resolved in this pass.
    pub resolved_this_pass: u32,
    /// Number of existing edges upgraded to higher confidence.
    pub upgraded_this_pass: u32,
    /// Number still unresolved after this pass.
    pub still_unresolved: u32,
    /// How long this pass took, in milliseconds.
    pub elapsed_ms: u64,
}

// ---------------------------------------------------------------------------
// BackgroundEnricher
// ---------------------------------------------------------------------------

/// Resolves `unresolved_refs` rows via LSP during idle time.
pub struct BackgroundEnricher {
    bridge: Arc<GraphBridge>,
    /// Minimum delay between LSP requests to avoid saturating the server.
    pub rate_limit: Duration,
    /// Cancellation flag — set via `cancel()`.
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

impl BackgroundEnricher {
    pub fn new(bridge: Arc<GraphBridge>) -> Self {
        Self {
            bridge,
            rate_limit: Duration::from_millis(100),
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Signal this enricher to stop at the next cancellation checkpoint.
    pub fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::Relaxed)
    }

    // -----------------------------------------------------------------------
    // Main enrichment pass — drain unresolved_refs
    // -----------------------------------------------------------------------

    /// Attempt to resolve up to `batch_size` rows from `unresolved_refs` via LSP.
    ///
    /// Rows are processed in descending order of how frequently their source
    /// symbol is referenced (high-traffic symbols first).  For each row the
    /// enricher:
    ///   1. Reads the source file once per file (batched).
    ///   2. Opens the file with `did_open` before the first LSP query.
    ///   3. Finds the column offset of `target_name` on `source_line` — the
    ///      actual reference site — then asks LSP "what is defined here?".
    ///   4. Closes the file with `did_close` after all refs in that file are
    ///      processed.
    pub async fn enrich_unresolved(&self, batch_size: usize) -> Result<EnrichmentProgress> {
        let start = Instant::now();

        // ----------------------------------------------------------------
        // Step 1: read a batch of unresolved refs + the current total.
        // ----------------------------------------------------------------
        struct UnresolvedRow {
            id: i64,
            source_id: i64,
            kind: String,
            source_line: Option<u32>,
            file_path: String,
            target_name: String,
        }

        let (rows, total_unresolved): (Vec<UnresolvedRow>, u32) = {
            let guard = self.bridge.db().lock().unwrap();

            let total: u32 = guard
                .conn
                .query_row("SELECT COUNT(*) FROM unresolved_refs", [], |r: &rusqlite::Row<'_>| {
                    r.get::<_, i64>(0)
                })
                .unwrap_or(0) as u32;

            // Select source_line (reference site) and target_name so we can
            // find the exact column where the reference appears.
            let mut stmt = guard.conn.prepare(
                "SELECT ur.id, ur.source_id, ur.kind, ur.source_line,
                        f.path       AS file_path,
                        ur.target_name
                 FROM unresolved_refs ur
                 JOIN symbols s ON s.id = ur.source_id
                 JOIN files   f ON f.id = s.file_id
                 ORDER BY (
                     SELECT COUNT(*) FROM edges WHERE target_id = ur.source_id
                 ) DESC
                 LIMIT ?1",
            )?;

            let batch: Vec<UnresolvedRow> = stmt
                .query_map([batch_size as i64], |row: &rusqlite::Row<'_>| {
                    Ok(UnresolvedRow {
                        id: row.get(0)?,
                        source_id: row.get(1)?,
                        kind: row.get(2)?,
                        source_line: row.get(3)?,
                        file_path: row.get(4)?,
                        target_name: row.get(5)?,
                    })
                })?
                .filter_map(|r: rusqlite::Result<UnresolvedRow>| r.ok())
                .collect();

            (batch, total)
        };

        // ----------------------------------------------------------------
        // Step 2: group rows by file_path for efficient file I/O + LSP
        // lifecycle management.
        // ----------------------------------------------------------------
        // Preserve insertion order so high-priority refs come first within
        // each file group.
        let mut by_file: HashMap<String, Vec<&UnresolvedRow>> = HashMap::new();
        let mut file_order: Vec<String> = Vec::new();
        for row in &rows {
            if !by_file.contains_key(&row.file_path) {
                file_order.push(row.file_path.clone());
            }
            by_file.entry(row.file_path.clone()).or_default().push(row);
        }

        let lsp = self.bridge.lsp();
        let workspace_root = self.bridge.workspace_root();
        let mut open_files: HashSet<String> = HashSet::new();
        let mut resolved = 0u32;

        // ----------------------------------------------------------------
        // Step 3: process each file group.
        // ----------------------------------------------------------------
        'files: for file_path in &file_order {
            if self.is_cancelled() {
                break;
            }

            // Read the file from disk once for the whole group.
            let full_path = workspace_root.join(file_path);
            let file_content = match std::fs::read_to_string(&full_path) {
                Ok(c) => c,
                Err(_) => continue, // can't read → skip group
            };

            // Open with LSP before any queries.
            if !open_files.contains(file_path) {
                // Ignore errors — if the server isn't available the LSP query
                // will also fail gracefully.
                let _ = lsp.did_open(file_path, &file_content).await;
                open_files.insert(file_path.clone());
            }

            let file_lines: Vec<&str> = file_content.lines().collect();

            for row in &by_file[file_path] {
                if self.is_cancelled() {
                    break 'files;
                }

                // Find the column where target_name appears on source_line.
                let ref_line = match row.source_line {
                    Some(l) => l,
                    None => 0,
                };
                let ref_col = GraphBridge::find_target_column(
                    workspace_root,
                    file_path,
                    ref_line,
                    &row.target_name,
                );

                // Verify the name is actually on that line before spending an
                // LSP round-trip.  find_target_column returns 0 both for
                // "found at column 0" and "not found".  Distinguish the two
                // by checking the line content directly.
                let name_on_line = file_lines
                    .get(ref_line as usize)
                    .map(|l| l.contains(row.target_name.as_str()))
                    .unwrap_or(false);

                if !name_on_line {
                    // target_name not on this line — skip without LSP call.
                    continue;
                }

                let result = self
                    .bridge
                    .resolve_definition_via_lsp(file_path, ref_line, ref_col)
                    .await;

                if let Ok(Some((target_id, _confidence))) = result {
                    let _ = self.bridge.persist_lsp_edge(
                        row.source_id,
                        target_id,
                        &row.kind,
                        row.source_line,
                        "enricher",
                    );

                    {
                        let guard = self.bridge.db().lock().unwrap();
                        let _ = guard
                            .conn
                            .execute("DELETE FROM unresolved_refs WHERE id = ?1", [row.id]);
                    }

                    resolved += 1;
                }

                tokio::time::sleep(self.rate_limit).await;
            }

            // Close the file after all refs in this file are done.
            let _ = lsp.did_close(file_path).await;
            open_files.remove(file_path);
        }

        // Close any files that were opened but not yet closed (e.g. after
        // early cancellation).
        for file_path in &open_files {
            let _ = lsp.did_close(file_path).await;
        }

        let still_unresolved: u32 = {
            let guard = self.bridge.db().lock().unwrap();
            guard
                .conn
                .query_row("SELECT COUNT(*) FROM unresolved_refs", [], |r: &rusqlite::Row<'_>| {
                    r.get::<_, i64>(0)
                })
                .unwrap_or(0) as u32
        };

        Ok(EnrichmentProgress {
            total_unresolved,
            resolved_this_pass: resolved,
            upgraded_this_pass: 0,
            still_unresolved,
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }

    // -----------------------------------------------------------------------
    // Confidence upgrade pass — re-confirm low-confidence tree-sitter edges
    // -----------------------------------------------------------------------

    /// For edges below `threshold` that have no LSP provenance, ask the LSP
    /// server to confirm them.  If confirmed, upgrade confidence to 1.0.
    ///
    /// The reference site is `edge.source_line` in the source symbol's file.
    /// The target name is taken from the target symbol's `name` column.
    pub async fn enrich_low_confidence(
        &self,
        threshold: f64,
        batch_size: usize,
    ) -> Result<EnrichmentProgress> {
        let start = Instant::now();

        struct LowConfRow {
            rowid: i64,
            source_id: i64,
            target_id: i64,
            kind: String,
            source_line: Option<u32>,
            file_path: String,
            target_name: String,
        }

        let rows: Vec<LowConfRow> = {
            let guard = self.bridge.db().lock().unwrap();
            // Join the target symbol to get its name (= what we search for on
            // source_line) and the source file path.
            let mut stmt = guard.conn.prepare(
                "SELECT e.rowid, e.source_id, e.target_id, e.kind, e.source_line,
                        f.path       AS file_path,
                        ts.name      AS target_name
                 FROM edges e
                 JOIN symbols s  ON s.id  = e.source_id
                 JOIN symbols ts ON ts.id = e.target_id
                 JOIN files   f  ON f.id  = s.file_id
                 WHERE e.confidence < ?1
                   AND e.rowid NOT IN (SELECT edge_rowid FROM lsp_edge_meta)
                 ORDER BY e.confidence ASC
                 LIMIT ?2",
            )?;

            let collected: Vec<LowConfRow> = stmt
                .query_map(
                    rusqlite::params![threshold, batch_size as i64],
                    |row: &rusqlite::Row<'_>| {
                        Ok(LowConfRow {
                            rowid: row.get(0)?,
                            source_id: row.get(1)?,
                            target_id: row.get(2)?,
                            kind: row.get(3)?,
                            source_line: row.get(4)?,
                            file_path: row.get(5)?,
                            target_name: row.get(6)?,
                        })
                    },
                )?
                .filter_map(|r: rusqlite::Result<LowConfRow>| r.ok())
                .collect();
            collected
        };

        // Group by file for efficient I/O and LSP lifecycle.
        let mut by_file: HashMap<String, Vec<&LowConfRow>> = HashMap::new();
        let mut file_order: Vec<String> = Vec::new();
        for row in &rows {
            if !by_file.contains_key(&row.file_path) {
                file_order.push(row.file_path.clone());
            }
            by_file.entry(row.file_path.clone()).or_default().push(row);
        }

        let lsp = self.bridge.lsp();
        let workspace_root = self.bridge.workspace_root();
        let mut open_files: HashSet<String> = HashSet::new();
        let mut upgraded = 0u32;

        'files: for file_path in &file_order {
            if self.is_cancelled() {
                break;
            }

            let full_path = workspace_root.join(file_path);
            let file_content = match std::fs::read_to_string(&full_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if !open_files.contains(file_path) {
                let _ = lsp.did_open(file_path, &file_content).await;
                open_files.insert(file_path.clone());
            }

            let file_lines: Vec<&str> = file_content.lines().collect();

            for row in &by_file[file_path] {
                if self.is_cancelled() {
                    break 'files;
                }

                let ref_line = row.source_line.unwrap_or(0);
                let ref_col = GraphBridge::find_target_column(
                    workspace_root,
                    file_path,
                    ref_line,
                    &row.target_name,
                );

                let name_on_line = file_lines
                    .get(ref_line as usize)
                    .map(|l| l.contains(row.target_name.as_str()))
                    .unwrap_or(false);

                if !name_on_line {
                    continue;
                }

                let result = self
                    .bridge
                    .resolve_definition_via_lsp(file_path, ref_line, ref_col)
                    .await;

                if let Ok(Some((lsp_target_id, _))) = result {
                    if lsp_target_id == row.target_id {
                        let did_upgrade = self.bridge.upgrade_confidence(
                            row.source_id,
                            row.target_id,
                            &row.kind,
                            1.0,
                        )?;

                        if did_upgrade {
                            let guard = self.bridge.db().lock().unwrap();
                            let _ = guard.conn.execute(
                                "INSERT OR REPLACE INTO lsp_edge_meta
                                 (edge_rowid, source, server, resolved_at)
                                 VALUES (?1, 'lsp', 'enricher', strftime('%s', 'now'))",
                                [row.rowid],
                            );
                            upgraded += 1;
                        }
                    }
                }

                tokio::time::sleep(self.rate_limit).await;
            }

            let _ = lsp.did_close(file_path).await;
            open_files.remove(file_path);
        }

        for file_path in &open_files {
            let _ = lsp.did_close(file_path).await;
        }

        Ok(EnrichmentProgress {
            total_unresolved: 0,
            resolved_this_pass: 0,
            upgraded_this_pass: upgraded,
            still_unresolved: 0,
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }

    // -----------------------------------------------------------------------
    // Sync entry point (kept for the stub's public signature)
    // -----------------------------------------------------------------------

    /// Synchronous wrapper around `enrich_unresolved`.
    ///
    /// Creates a single-threaded Tokio runtime, runs one enrichment pass, and
    /// returns the number of refs resolved.
    pub fn run_batch(&self, _db: &Database, batch_size: usize) -> Result<u64> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        let progress = rt.block_on(self.enrich_unresolved(batch_size))?;
        Ok(progress.resolved_this_pass as u64)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "enricher_tests.rs"]
mod tests;
