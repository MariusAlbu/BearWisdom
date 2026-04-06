// =============================================================================
// indexer/full.rs  —  full index pipeline
//
// Pipeline:
//   1. Walk the project tree (respect .gitignore) via changeset::full_scan.
//   2. Read + hash + parse each file with tree-sitter (parallel via Rayon).
//   3. Write files + symbols via shared write pipeline.
//   4. Run cross-file resolution (match unresolved refs to symbol IDs).
//   5. Index content for FTS5 + chunk for embeddings.
//   6. Run connector registry + non-flow post-steps.
//   7. Store indexed_commit in metadata (for git-aware reindex).
// =============================================================================

use crate::db::Database;
use crate::indexer::changeset;
use crate::indexer::ref_cache::RefCache;
use crate::indexer::resolve;
use crate::indexer::write;
use crate::languages::{self, LanguageRegistry};
use crate::types::{IndexStats, ParsedFile};
use crate::walker::WalkedFile;
use anyhow::{Context, Result};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::{debug, info, warn};

/// Progress callback invoked at each pipeline step.
///
/// Arguments: `(step_label, progress_0_to_1, optional_detail_text)`
///
/// Step labels: `"scanning"`, `"parsing"`, `"resolving"`, `"indexing_content"`,
/// `"connectors"`.  Callers may also emit their own labels after `full_index`
/// returns (e.g. `"concepts"`, `"embedding"`).
pub type ProgressFn = Box<dyn Fn(&str, f64, Option<&str>) + Send>;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Index all source files under `project_root` and write the results to `db`.
///
/// This is a full (non-incremental) index: existing data for re-indexed files
/// is deleted via the CASCADE constraint and replaced.
///
/// `progress` is an optional callback invoked at each pipeline phase boundary.
/// Pass `None` to suppress progress notifications (CLI, tests).
///
/// `pre_walked` allows the caller to supply an already-walked file list (e.g.
/// from `bearwisdom_profile::walk_files` performed during project scanning)
/// to avoid a redundant directory traversal.  Pass `None` to walk inline.
pub fn full_index(
    db: &mut Database,
    project_root: &Path,
    progress: Option<ProgressFn>,
    pre_walked: Option<Vec<WalkedFile>>,
    ref_cache: Option<&Arc<Mutex<RefCache>>>,
) -> Result<IndexStats> {
    let emit = |step: &str, pct: f64, detail: Option<&str>| {
        if let Some(ref cb) = progress {
            cb(step, pct, detail);
        }
    };

    let start = Instant::now();
    info!("Starting full index of {}", project_root.display());

    // --- Step 1: Change detection (FullScan) ---
    emit("scanning", 0.0, None);
    let cs = changeset::full_scan(project_root, pre_walked)?;
    let file_count = cs.added.len();
    info!("Found {} source files", file_count);
    emit("scanning", 1.0, Some(&format!("{} files found", file_count)));

    // --- Step 1b: Clear existing data ---
    // For full reindex: DROP + CREATE core tables instead of DELETE.
    // DELETE on a large indexed table is O(n log n) due to index maintenance;
    // DROP + CREATE is O(1) and lets SQLite reclaim pages immediately.
    // Virtual tables (symbols_fts, fts_content, vec_chunks) are handled
    // separately to avoid leaving their internal state pointing at stale rowids.
    {
        let count: i64 = db.conn().query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0)).unwrap_or(0);
        if count > 0 {
            info!("Dropping and recreating index tables for full rebuild ({} existing files)", count);

            // Drop vec_chunks first (virtual table — not CASCADE-covered).
            if crate::search::vector_store::vec_table_exists(db.conn()) {
                let _ = db.conn().execute_batch("DELETE FROM vec_chunks");
            }

            // Drop the FTS trigger + virtual table so their internal rowid state
            // doesn't point at stale symbols after we drop and recreate symbols.
            // The triggers and table will be recreated by create_schema below.
            let _ = db.conn().execute_batch(
                "DROP TRIGGER IF EXISTS symbols_ai;
                 DROP TRIGGER IF EXISTS symbols_ad;
                 DROP TRIGGER IF EXISTS symbols_au;
                 DROP TABLE IF EXISTS symbols_fts;",
            );

            // Drop core tables (FK-ordered: dependents first).
            // Disable FK enforcement so we can drop in any order.
            // Derived tables (routes, flow_edges, connection_points, db_mappings,
            // code_chunks, lsp_edge_meta) must also be cleared — they reference
            // file/symbol IDs that become stale after DROP TABLE files/symbols.
            let _ = db.conn().execute_batch(
                "PRAGMA foreign_keys = OFF;
                 DROP TABLE IF EXISTS lsp_edge_meta;
                 DROP TABLE IF EXISTS flow_edges;
                 DROP TABLE IF EXISTS connection_points;
                 DROP TABLE IF EXISTS routes;
                 DROP TABLE IF EXISTS db_mappings;
                 DROP TABLE IF EXISTS code_chunks;
                 DROP TABLE IF EXISTS edges;
                 DROP TABLE IF EXISTS imports;
                 DROP TABLE IF EXISTS unresolved_refs;
                 DROP TABLE IF EXISTS external_refs;
                 DROP TABLE IF EXISTS symbols;
                 DROP TABLE IF EXISTS files;
                 PRAGMA foreign_keys = ON;",
            );

            // Recreate all tables, indexes, triggers, and virtual tables
            // using the canonical schema.
            crate::db::schema::create_schema(db.conn())
                .context("Failed to recreate schema after drop")?;

            info!("Index tables recreated");
        }
    }

    // --- Steps 2-3: Read + parse (parallel via Rayon) ---
    let registry = languages::default_registry();
    let files = cs.added; // FullScan puts everything in `added`
    emit("parsing", 0.0, Some(&format!("0/{} files", files.len())));
    let results: Vec<Result<ParsedFile>> =
        files.par_iter().map(|w| parse_file(w, registry)).collect();

    let mut parsed: Vec<ParsedFile> = Vec::with_capacity(files.len());
    let mut files_with_errors = 0u32;

    for (walked, result) in files.iter().zip(results) {
        match result {
            Ok(pf) => {
                if pf.has_errors {
                    files_with_errors += 1;
                    debug!("Syntax errors in {}", walked.relative_path);
                }
                parsed.push(pf);
            }
            Err(e) => {
                warn!("Failed to parse {}: {e}", walked.relative_path);
            }
        }
    }
    info!("Parsed {} files ({} with syntax errors)", parsed.len(), files_with_errors);
    emit("parsing", 1.0, Some(&format!("{} files parsed", parsed.len())));

    // --- Step 4: Write files + symbols (shared pipeline) ---
    let (file_id_map, symbol_id_map) =
        write::write_parsed_files(db, &parsed).context("Failed to write index to database")?;
    info!(
        "Wrote {} symbols across {} files",
        symbol_id_map.len(),
        file_id_map.len()
    );

    // --- Step 5: Cross-file resolution + edge writing ---
    emit("resolving", 0.0, None);
    let project_ctx = super::project_context::build_project_context(project_root);
    let rstats = resolve::resolve_and_write(db, &parsed, &symbol_id_map, Some(&project_ctx))
        .context("Failed to resolve references")?;
    info!(
        "Wrote {} edges, {} external, {} unresolved references",
        rstats.resolved, rstats.external, rstats.unresolved
    );
    emit("resolving", 1.0, Some(&format!("{} edges resolved", rstats.resolved)));

    // --- Step 6a: FTS content index (shared pipeline) ---
    emit("indexing_content", 0.0, Some("Building search index"));
    let fts_count = write::update_fts_content(db, &parsed, &file_id_map)?;
    info!("Indexed {} files for FTS5 content search", fts_count);

    // --- Step 6b: Code chunking (shared pipeline) ---
    let total_chunks = write::update_chunks(db, &parsed, &file_id_map, true)?;
    info!("Created {total_chunks} code chunks");
    emit("indexing_content", 1.0, Some(&format!("{total_chunks} chunks created")));

    // --- Step 7a: Flow connectors (registry pipeline) ---
    //
    // All cross-framework flow connectors run through the ConnectorRegistry:
    //   detect → extract ConnectionPoints → match start↔stop → write flow_edges
    //
    // 18 connectors: REST, gRPC, MQ, GraphQL, events, IPC (Tauri + Electron),
    // DI (.NET + Angular + Spring), routes (Spring, Django, FastAPI, Go, Rails,
    // Laravel, NestJS, Next.js).
    emit("connectors", 0.0, Some("Running connectors"));
    let connector_start = Instant::now();

    // Enrich routes written by tree-sitter extractors (set resolved_route where NULL).
    if let Err(e) = db.conn().execute(
        "UPDATE routes SET resolved_route = route_template WHERE resolved_route IS NULL",
        [],
    ) {
        warn!("Route enrichment failed: {e}");
    }

    let connector_registry = crate::connectors::registry::build_default_registry();
    match connector_registry.run(db.conn(), project_root, &project_ctx) {
        Ok(flow_count) => info!(
            "Connectors: {flow_count} flow edges in {:.2}s",
            connector_start.elapsed().as_secs_f64()
        ),
        Err(e) => warn!("Connector registry failed: {e}"),
    }

    // --- Step 7b: Non-flow post-index hooks ---
    //
    // These write to tables other than flow_edges (db_mappings, concepts) so
    // they don't fit the ConnectionPoint → flow_edge pipeline.  Called directly.
    if let Err(e) = crate::connectors::ef_core::connect(db) {
        warn!("EF Core connector: {e}");
    }
    if project_ctx.python_packages.contains("django") {
        if let Err(e) = crate::connectors::django::connect(db, project_root) {
            warn!("Django connector: {e}");
        }
    }
    run_react_patterns(db.conn(), project_root);

    emit("connectors", 1.0, None);

    // ANALYZE for query planner accuracy.
    if let Err(e) = db.conn().execute("ANALYZE", []) {
        warn!("ANALYZE failed (non-fatal): {e}");
    }

    // --- Step 8: Store indexed commit for git-aware reindex ---
    if let Some(commit) = cs.commit {
        if let Err(e) = changeset::set_meta(db, "indexed_commit", &commit) {
            warn!("Failed to store indexed_commit: {e}");
        }
    }

    let duration = start.elapsed();

    let stats = read_stats(db.conn(), files_with_errors, duration.as_millis() as u64)?;
    info!(
        "Full index complete in {:.2}s: {} files, {} symbols, {} edges, {} routes, {} db_mappings",
        duration.as_secs_f64(),
        stats.file_count,
        stats.symbol_count,
        stats.edge_count,
        stats.route_count,
        stats.db_mapping_count,
    );

    // Populate the pool-level ref cache (if the caller supplied one) so
    // incremental reindex can skip re-parsing unchanged dependent files on the
    // next pass.  The lock is held only long enough to drain parsed into the
    // cache; the pool connection that ran full_index is irrelevant after this.
    if let Some(rc) = ref_cache {
        let mut guard = rc.lock().unwrap();
        guard.store_all(&parsed);
        debug!("RefCache populated: {} files", parsed.len());
    }

    Ok(stats)
}

// ---------------------------------------------------------------------------
// Parse a single file
// ---------------------------------------------------------------------------

pub(crate) fn parse_file(walked: &WalkedFile, registry: &LanguageRegistry) -> Result<ParsedFile> {
    let bytes = std::fs::read(&walked.absolute_path)
        .with_context(|| format!("Cannot read {}", walked.relative_path))?;

    // SHA-256 of the raw bytes for change detection.
    let hash = {
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        format!("{:x}", hasher.finalize())
    };

    let content = String::from_utf8(bytes)
        .with_context(|| format!("Non-UTF-8 content in {}", walked.relative_path))?;

    let size = content.len() as u64;
    let line_count = content.lines().count() as u32;

    // Capture mtime for fast change detection on next incremental pass.
    let mtime = std::fs::metadata(&walked.absolute_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    // Dispatch to the language plugin (dedicated or generic fallback).
    let r = registry.get(walked.language).extract(
        &content,
        &walked.relative_path,
        walked.language,
    );

    Ok(ParsedFile {
        path: walked.relative_path.clone(),
        language: walked.language.to_string(),
        content_hash: hash,
        size,
        line_count,
        mtime,
        symbols: r.symbols,
        refs: r.refs,
        routes: r.routes,
        db_sets: r.db_sets,
        content: Some(content),
        has_errors: r.has_errors,
    })
}

// ---------------------------------------------------------------------------
// Non-flow connector helpers
// ---------------------------------------------------------------------------

fn run_react_patterns(conn: &rusqlite::Connection, project_root: &Path) {
    match crate::connectors::react_patterns::find_zustand_stores(conn, project_root) {
        Ok(stores) => {
            match crate::connectors::react_patterns::find_story_mappings(conn, project_root) {
                Ok(stories) if !stores.is_empty() || !stories.is_empty() => {
                    let _ = crate::connectors::react_patterns::create_react_concepts(conn, &stores, &stories)
                        .map_err(|e| warn!("React concept creation: {e}"));
                }
                Err(e) => warn!("Story mapping: {e}"),
                _ => {}
            }
        }
        Err(e) => warn!("Zustand store detection: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

pub(crate) fn read_stats(
    conn: &rusqlite::Connection,
    files_with_errors: u32,
    duration_ms: u64,
) -> Result<IndexStats> {
    let file_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
    let symbol_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))?;
    let edge_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
    let unresolved_ref_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM unresolved_refs", [], |r| r.get(0))?;
    let external_ref_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM external_refs", [], |r| r.get(0))?;
    let route_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM routes", [], |r| r.get(0))?;
    let db_mapping_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM db_mappings", [], |r| r.get(0))?;

    let flow_edge_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM flow_edges", [], |r| r.get(0))?;

    Ok(IndexStats {
        file_count,
        symbol_count,
        edge_count,
        unresolved_ref_count,
        external_ref_count,
        route_count,
        db_mapping_count,
        flow_edge_count,
        files_with_errors,
        duration_ms,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "full_tests.rs"]
mod tests;
