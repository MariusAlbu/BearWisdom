// =============================================================================
// indexer/full.rs  —  full index pipeline
//
// Pipeline:
//   1. Walk the project tree (respect .gitignore).
//   2. Read + hash each file.
//   3. Parse with tree-sitter (per-language extractor).
//   4. Write files + symbols to SQLite in a single transaction.
//   5. Run cross-file resolution (match unresolved refs to symbol IDs).
//   6. Write resolved edges; log unresolved refs for diagnostics.
//   7. Run connector registry (extract→match→flow_edges) + non-flow post-steps.
//
// Performance notes:
//   Steps 2-3 are run sequentially for simplicity.  The tree-sitter Parser
//   is NOT thread-safe, so parallel parsing would require one Parser instance
//   per thread (via rayon::ThreadLocal).  For the benchmark target (~500 files,
//   ~100K LOC) sequential is fast enough; adding rayon is a straightforward
//   follow-up if needed.
// =============================================================================

use crate::db::Database;
use crate::indexer::resolve;
use crate::languages::{self, LanguageRegistry};
use crate::types::{IndexStats, ParsedFile};
use crate::walker::{self, WalkedFile};
use anyhow::{Context, Result};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
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
) -> Result<IndexStats> {
    let emit = |step: &str, pct: f64, detail: Option<&str>| {
        if let Some(ref cb) = progress {
            cb(step, pct, detail);
        }
    };

    let start = Instant::now();
    info!("Starting full index of {}", project_root.display());

    // --- Step 1: Walk ---
    emit("scanning", 0.0, None);
    let files = match pre_walked {
        Some(f) => {
            info!("Using pre-walked file list ({} files)", f.len());
            f
        }
        None => walker::walk(project_root)
            .with_context(|| format!("Failed to walk {}", project_root.display()))?,
    };
    info!("Found {} source files", files.len());
    emit("scanning", 1.0, Some(&format!("{} files found", files.len())));

    // --- Step 1b: Clear existing data ---
    // For full reindex: DROP + CREATE core tables instead of DELETE.
    // DELETE on a large indexed table is O(n log n) due to index maintenance;
    // DROP + CREATE is O(1) and lets SQLite reclaim pages immediately.
    // Virtual tables (symbols_fts, fts_content, vec_chunks) are handled
    // separately to avoid leaving their internal state pointing at stale rowids.
    {
        let count: i64 = db.conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0)).unwrap_or(0);
        if count > 0 {
            info!("Dropping and recreating index tables for full rebuild ({} existing files)", count);

            // Drop vec_chunks first (virtual table — not CASCADE-covered).
            if crate::search::vector_store::vec_table_exists(&db.conn) {
                let _ = db.conn.execute_batch("DELETE FROM vec_chunks");
            }

            // Drop the FTS trigger + virtual table so their internal rowid state
            // doesn't point at stale symbols after we drop and recreate symbols.
            // The triggers and table will be recreated by create_schema below.
            let _ = db.conn.execute_batch(
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
            let _ = db.conn.execute_batch(
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
            crate::db::schema::create_schema(&db.conn)
                .context("Failed to recreate schema after drop")?;

            info!("Index tables recreated");
        }
    }

    // --- Steps 2-3: Read + parse (parallel via Rayon) ---
    // Build the language registry once — each plugin provides its grammar,
    // scope config, and extraction logic.  parse_file delegates to the plugin.
    let registry = languages::default_registry();
    emit("parsing", 0.0, Some(&format!("0/{} files", files.len())));
    let results: Vec<Result<ParsedFile>> =
        files.par_iter().map(|w| parse_file(w, &registry)).collect();

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

    // --- Step 4: Write files + symbols ---
    let (file_id_map, symbol_id_map) =
        write_to_db(db, &parsed).context("Failed to write index to database")?;
    info!(
        "Wrote {} symbols across {} files",
        symbol_id_map.len(),
        file_id_map.len()
    );

    // --- Step 5-6: Cross-file resolution + edge writing ---
    emit("resolving", 0.0, None);
    let project_ctx = super::project_context::build_project_context(project_root);
    let rstats = resolve::resolve_and_write(db, &parsed, &symbol_id_map, Some(&project_ctx))
        .context("Failed to resolve references")?;
    info!(
        "Wrote {} edges, {} external, {} unresolved references",
        rstats.resolved, rstats.external, rstats.unresolved
    );
    emit("resolving", 1.0, Some(&format!("{} edges resolved", rstats.resolved)));

    // --- Step 4b: Index file content for FTS5 trigram search ---
    emit("indexing_content", 0.0, Some("Building search index"));
    {
        let content_entries: Vec<(i64, &str, &str)> = parsed
            .iter()
            .filter_map(|pf| {
                let file_id = file_id_map.get(&pf.path)?;
                let content = pf.content.as_deref()?;
                Some((*file_id, pf.path.as_str(), content))
            })
            .collect();

        match crate::search::content_index::batch_index_content(
            &db.conn,
            &content_entries,
        ) {
            Ok(n) => info!("Indexed {n} files for FTS5 content search"),
            Err(e) => warn!("FTS5 content indexing failed: {e}"),
        }
    }

    // --- Step 4c: Chunk files for embedding (vectors created later) ---
    let mut total_chunks = 0u32;
    for pf in &parsed {
        if let (Some(&file_id), Some(content)) =
            (file_id_map.get(&pf.path), pf.content.as_deref())
        {
            match crate::search::chunker::chunk_and_store(&db.conn, file_id, content) {
                Ok(n) => total_chunks += n,
                Err(e) => debug!("Failed to chunk {}: {e}", pf.path),
            }
        }
    }
    info!("Created {total_chunks} code chunks");

    emit("indexing_content", 1.0, Some(&format!("{total_chunks} chunks created")));

    // --- Step 7: Connectors (registry-based) ---
    //
    // All connectors run through the ConnectorRegistry pipeline:
    //   1. detect  — filter to connectors relevant for this project
    //   2. extract — collect ConnectionPoints (start=caller, stop=handler)
    //   3. match   — group by protocol, resolve start↔stop pairs
    //   4. write   — insert flow_edges + back-fill legacy routes table
    //
    // Non-flow connectors (ef_core, react_patterns) run as standalone post-steps.
    emit("connectors", 0.0, Some("Running connectors"));
    let connector_start = Instant::now();

    // Enrich routes written by tree-sitter extractors (set resolved_route where NULL).
    if let Err(e) = db.conn.execute(
        "UPDATE routes SET resolved_route = route_template WHERE resolved_route IS NULL",
        [],
    ) {
        warn!("Route enrichment failed: {e}");
    }

    let registry = crate::connectors::registry::build_default_registry();
    match registry.run(&db.conn, project_root, &project_ctx) {
        Ok(flow_count) => info!(
            "Connectors: {flow_count} flow edges in {:.2}s",
            connector_start.elapsed().as_secs_f64()
        ),
        Err(e) => warn!("Connector registry failed: {e}"),
    }

    // Non-flow post-steps: EF Core (DB mappings), Django (views/models), React patterns.
    if let Err(e) = crate::connectors::ef_core::connect(db) {
        warn!("EF Core connector: {e}");
    }
    if project_ctx.python_packages.contains("django") {
        if let Err(e) = crate::connectors::django::connect(db, project_root) {
            warn!("Django connector: {e}");
        }
    }
    run_react_patterns(&db.conn, project_root);

    emit("connectors", 1.0, None);

    // Update SQLite's statistics so the query planner has accurate selectivity
    // data for all of the new covering indexes.  ANALYZE is fast on a freshly
    // written database (sequential scan, no I/O amplification).
    if let Err(e) = db.conn.execute("ANALYZE", []) {
        warn!("ANALYZE failed (non-fatal): {e}");
    }

    let duration = start.elapsed();

    // Read back counts for the stats report.
    let stats = read_stats(&db.conn, files_with_errors, duration.as_millis() as u64)?;
    info!(
        "Full index complete in {:.2}s: {} files, {} symbols, {} edges, {} routes, {} db_mappings",
        duration.as_secs_f64(),
        stats.file_count,
        stats.symbol_count,
        stats.edge_count,
        stats.route_count,
        stats.db_mapping_count,
    );

    // Populate the ref cache (if the caller opted in) so incremental reindex
    // can skip re-parsing unchanged dependent files on the next pass.
    if let Some(ref_cache) = db.ref_cache.as_mut() {
        ref_cache.store_all(&parsed);
        tracing::debug!("RefCache populated: {} files", parsed.len());
    }

    Ok(stats)
}

// ---------------------------------------------------------------------------
// Parse a single file
// ---------------------------------------------------------------------------

pub(crate) fn parse_file(walked: &WalkedFile, registry: &LanguageRegistry) -> Result<ParsedFile> {
    let content = std::fs::read_to_string(&walked.absolute_path)
        .with_context(|| format!("Cannot read {}", walked.relative_path))?;

    // SHA-256 of the raw bytes for change detection.
    let hash = {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    };

    let size = content.len() as u64;
    let line_count = content.lines().count() as u32;

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
        symbols: r.symbols,
        refs: r.refs,
        routes: r.routes,
        db_sets: r.db_sets,
        content: Some(content),
        has_errors: r.has_errors,
    })
}

// ---------------------------------------------------------------------------
// Database writes
// ---------------------------------------------------------------------------

/// Write all parsed files and their symbols in a single transaction.
///
/// Returns two maps:
///   - `file_id_map`: relative path → SQLite file row ID
///   - `symbol_id_map`: (relative_path, qualified_name) → symbol row ID
///
/// The symbol_id_map is used by the resolver to link unresolved references to
/// symbol IDs without doing expensive per-ref SQL queries.
fn write_to_db(
    db: &mut Database,
    parsed: &[ParsedFile],
) -> Result<(HashMap<String, i64>, HashMap<(String, String), i64>)> {
    let conn = &db.conn;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let tx = conn.unchecked_transaction().context("Failed to begin transaction")?;

    let mut file_id_map: HashMap<String, i64> = HashMap::new();
    let mut symbol_id_map: HashMap<(String, String), i64> = HashMap::new();

    // Prepare statements once — avoids SQL re-parsing on every row.
    // CachedStatement borrows `tx`, so we reborrow as needed inside the loop.
    for pf in parsed {
        // Upsert the file row (delete existing symbols via CASCADE, then re-insert).
        tx.prepare_cached(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET
               hash = excluded.hash,
               language = excluded.language,
               last_indexed = excluded.last_indexed",
        )
        .context("Failed to prepare file upsert")?
        .execute(rusqlite::params![pf.path, pf.content_hash, pf.language, now])
        .with_context(|| format!("Failed to upsert file {}", pf.path))?;

        // If it was an UPDATE the last_insert_rowid() returns 0 on some platforms.
        // Re-fetch by path to be safe.
        let file_id: i64 = tx
            .prepare_cached("SELECT id FROM files WHERE path = ?1")
            .context("Failed to prepare file id select")?
            .query_row([&pf.path], |r| r.get(0))
            .with_context(|| format!("Failed to get file_id for {}", pf.path))?;

        file_id_map.insert(pf.path.clone(), file_id);

        // Delete existing symbols for this file so we can re-insert cleanly.
        // (The ON CONFLICT above updates the file row but doesn't cascade-delete symbols.)
        tx.prepare_cached("DELETE FROM symbols WHERE file_id = ?1")
            .context("Failed to prepare symbol delete")?
            .execute([file_id])?;

        // Delete existing imports for this file (not cascaded by symbols delete).
        tx.prepare_cached("DELETE FROM imports WHERE file_id = ?1")
            .context("Failed to prepare import delete")?
            .execute([file_id])?;

        // Insert all symbols for this file.
        for sym in &pf.symbols {
            tx.prepare_cached(
                "INSERT INTO symbols
                   (file_id, name, qualified_name, kind, line, col,
                    end_line, end_col, scope_path, signature, doc_comment, visibility)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )
            .context("Failed to prepare symbol insert")?
            .execute(rusqlite::params![
                file_id,
                sym.name,
                sym.qualified_name,
                sym.kind.as_str(),
                sym.start_line,
                sym.start_col,
                sym.end_line,
                sym.end_col,
                sym.scope_path,
                sym.signature,
                sym.doc_comment,
                sym.visibility.map(|v| v.as_str()),
            ])
            .with_context(|| format!("Failed to insert symbol {} in {}", sym.qualified_name, pf.path))?;

            let sym_id = tx.last_insert_rowid();
            symbol_id_map.insert((pf.path.clone(), sym.qualified_name.clone()), sym_id);
        }

        // Insert route records for this file (ASP.NET [HttpGet], [Route], etc.).
        for route in &pf.routes {
            let sym_id = symbol_id_map
                .get(&(
                    pf.path.clone(),
                    pf.symbols
                        .get(route.handler_symbol_index)
                        .map(|s| s.qualified_name.clone())
                        .unwrap_or_default(),
                ))
                .copied();

            tx.prepare_cached(
                "INSERT OR IGNORE INTO routes
                   (file_id, symbol_id, http_method, route_template, line)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .context("Failed to prepare route insert")?
            .execute(rusqlite::params![
                file_id,
                sym_id,
                route.http_method,
                route.template,
                pf.symbols.get(route.handler_symbol_index).map(|s| s.start_line),
            ])
            .with_context(|| format!("Failed to insert route for {}", pf.path))?;
        }

        // Insert import records for this file.
        // Any ref with EdgeKind::Imports is a `using` (C#) or `import` (TS) directive.
        // For C#: imported_name = module_path = "eShop.Catalog.API.Model", alias = NULL
        // For TS:  imported_name = "Foo", module_path = "./bar", alias = NULL
        for r in &pf.refs {
            if r.kind != crate::types::EdgeKind::Imports {
                continue;
            }
            let imported_name = &r.target_name;
            let module_path = r.module.as_deref();
            // Use module as module_path; for C# both target_name and module hold the namespace.
            tx.prepare_cached(
                "INSERT INTO imports (file_id, imported_name, module_path, alias, line)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .context("Failed to prepare import insert")?
            .execute(rusqlite::params![
                file_id,
                imported_name,
                module_path,
                Option::<&str>::None, // alias extraction not yet implemented
                r.line,
            ])
            .with_context(|| format!("Failed to insert import '{}' in {}", imported_name, pf.path))?;
        }
    }

    tx.commit().context("Failed to commit file/symbol transaction")?;
    Ok((file_id_map, symbol_id_map))
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

    Ok(IndexStats {
        file_count,
        symbol_count,
        edge_count,
        unresolved_ref_count,
        external_ref_count,
        route_count,
        db_mapping_count,
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
