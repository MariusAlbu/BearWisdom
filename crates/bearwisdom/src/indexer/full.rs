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
//   7. Run HTTP and EF Core connectors (post-processing).
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
use crate::parser::extractors::{csharp, generic, go, java, python, rust, typescript};
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
            let _ = db.conn.execute_batch(
                "PRAGMA foreign_keys = OFF;
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
    // Each file gets its own tree-sitter Parser inside parse_file() — Parser is
    // not Send, but creating one per closure call is cheap and safe.
    emit("parsing", 0.0, Some(&format!("0/{} files", files.len())));
    let results: Vec<Result<ParsedFile>> = files.par_iter().map(parse_file).collect();

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

    // --- Step 7: Connectors (parallel) ---
    //
    // 21 connectors split across 4 rayon threads.  Each thread opens its own
    // Database connection — WAL mode + busy_timeout serialise writes while
    // allowing concurrent reads.  Connectors are grouped by ecosystem so
    // related work stays on the same connection.
    emit("connectors", 0.0, Some("Running connectors"));
    let connector_start = Instant::now();

    match db.path.as_deref() {
        Some(db_path) => {
            let root = project_root.to_path_buf();
            let path = db_path.to_path_buf();

            rayon::scope(|s| {
                // --- Group 1: .NET stack ---
                let root1 = root.clone();
                let path1 = path.clone();
                s.spawn(move |_| {
                    let Ok(tdb) = crate::db::Database::open(&path1) else { return };
                    if let Err(e) = crate::connectors::http_api::connect(&tdb) {
                        warn!("HTTP API connector failed: {e}");
                    }
                    if let Err(e) = crate::connectors::ef_core::connect(&tdb) {
                        warn!("EF Core connector failed: {e}");
                    }
                    match crate::connectors::dotnet_http_client::connect(&tdb.conn, &root1) {
                        Ok(n) => if n > 0 { info!(".NET HTTP client connector: {n} routes matched") },
                        Err(e) => warn!(".NET HTTP client connector failed: {e}"),
                    }
                    if let Err(e) = crate::connectors::grpc::connect(&tdb) {
                        warn!("gRPC connector failed: {e}");
                    }
                    match crate::connectors::dotnet_di::detect_di_registrations(&tdb.conn, &root1) {
                        Ok(registrations) => {
                            if !registrations.is_empty() {
                                match crate::connectors::dotnet_di::link_di_registrations(&tdb.conn, &registrations) {
                                    Ok(linked) => info!("DI connector: {} registrations, {} edges", registrations.len(), linked),
                                    Err(e) => warn!("DI registration linking failed: {e}"),
                                }
                            }
                        }
                        Err(e) => warn!("DI registration detection failed: {e}"),
                    }
                    match crate::connectors::dotnet_events::find_integration_events(&tdb.conn) {
                        Ok(events) => {
                            match crate::connectors::dotnet_events::find_event_handlers(&tdb.conn, &root1) {
                                Ok(handlers) => {
                                    if !events.is_empty() && !handlers.is_empty() {
                                        match crate::connectors::dotnet_events::link_events_to_handlers(&tdb.conn, &events, &handlers) {
                                            Ok(linked) => info!("Events connector: {} events, {} handlers, {} edges", events.len(), handlers.len(), linked),
                                            Err(e) => warn!("Event linking failed: {e}"),
                                        }
                                    }
                                }
                                Err(e) => warn!("Event handler detection failed: {e}"),
                            }
                        }
                        Err(e) => warn!("Integration event detection failed: {e}"),
                    }
                });

                // --- Group 2: Frontend ---
                let root2 = root.clone();
                let path2 = path.clone();
                s.spawn(move |_| {
                    let Ok(tdb) = crate::db::Database::open(&path2) else { return };
                    match crate::connectors::frontend_http::detect_http_calls(&tdb.conn, &root2) {
                        Ok(http_calls) => {
                            if !http_calls.is_empty() {
                                match crate::connectors::frontend_http::match_http_calls_to_routes(&tdb.conn, &http_calls) {
                                    Ok(matched) => info!("Frontend HTTP: {} calls detected, {} matched", http_calls.len(), matched),
                                    Err(e) => warn!("Frontend HTTP route matching failed: {e}"),
                                }
                            }
                        }
                        Err(e) => warn!("Frontend HTTP detection failed: {e}"),
                    }
                    match crate::connectors::tauri_ipc::connect(&tdb.conn, &root2) {
                        Ok(()) => info!("Tauri IPC connector complete"),
                        Err(e) => warn!("Tauri IPC connector failed: {e}"),
                    }
                    match crate::connectors::react_patterns::find_zustand_stores(&tdb.conn, &root2) {
                        Ok(stores) => {
                            match crate::connectors::react_patterns::find_story_mappings(&tdb.conn, &root2) {
                                Ok(stories) => {
                                    if !stores.is_empty() || !stories.is_empty() {
                                        match crate::connectors::react_patterns::create_react_concepts(&tdb.conn, &stores, &stories) {
                                            Ok(()) => info!("React patterns: {} stores, {} stories", stores.len(), stories.len()),
                                            Err(e) => warn!("React concept creation failed: {e}"),
                                        }
                                    }
                                }
                                Err(e) => warn!("Story mapping detection failed: {e}"),
                            }
                        }
                        Err(e) => warn!("Zustand store detection failed: {e}"),
                    }
                    match crate::connectors::electron_ipc::connect(&tdb, &root2) {
                        Ok(()) => info!("Electron IPC connector complete"),
                        Err(e) => warn!("Electron IPC connector failed: {e}"),
                    }
                    match crate::connectors::angular_di::connect(&tdb.conn, &root2) {
                        Ok(n) => if n > 0 { info!("Angular DI connector: {n} flow edges") },
                        Err(e) => warn!("Angular DI connector failed: {e}"),
                    }
                });

                // --- Group 3: JVM + Python ---
                let root3 = root.clone();
                let path3 = path.clone();
                s.spawn(move |_| {
                    let Ok(tdb) = crate::db::Database::open(&path3) else { return };
                    match crate::connectors::spring::find_spring_routes(&tdb.conn, &root3) {
                        Ok(routes) => {
                            match crate::connectors::spring::find_spring_services(&tdb.conn, &root3) {
                                Ok(services) => {
                                    if !routes.is_empty() || !services.is_empty() {
                                        match crate::connectors::spring::register_spring_patterns(&tdb.conn, &routes, &services) {
                                            Ok(()) => info!("Spring connector: {} routes, {} services", routes.len(), services.len()),
                                            Err(e) => warn!("Spring pattern registration failed: {e}"),
                                        }
                                    }
                                }
                                Err(e) => warn!("Spring service detection failed: {e}"),
                            }
                        }
                        Err(e) => warn!("Spring route detection failed: {e}"),
                    }
                    match crate::connectors::spring_di::connect(&tdb.conn, &root3) {
                        Ok(n) => if n > 0 { info!("Spring DI connector: {n} flow edges") },
                        Err(e) => warn!("Spring DI connector failed: {e}"),
                    }
                    match crate::connectors::django::connect(&tdb, &root3) {
                        Ok(()) => info!("Django connector complete"),
                        Err(e) => warn!("Django connector failed: {e}"),
                    }
                    match crate::connectors::fastapi_routes::connect(&tdb.conn, &root3) {
                        Ok(n) => if n > 0 { info!("FastAPI routes connector: {n} routes") },
                        Err(e) => warn!("FastAPI routes connector failed: {e}"),
                    }
                });

                // --- Group 4: Other frameworks ---
                let root4 = root.clone();
                let path4 = path.clone();
                s.spawn(move |_| {
                    let Ok(tdb) = crate::db::Database::open(&path4) else { return };
                    match crate::connectors::graphql::connect(&tdb, &root4) {
                        Ok(()) => info!("GraphQL connector complete"),
                        Err(e) => warn!("GraphQL connector failed: {e}"),
                    }
                    match crate::connectors::message_queue::connect(&tdb, &root4) {
                        Ok(()) => info!("Message queue connector complete"),
                        Err(e) => warn!("Message queue connector failed: {e}"),
                    }
                    match crate::connectors::go_routes::connect(&tdb.conn, &root4) {
                        Ok(n) => if n > 0 { info!("Go routes connector: {n} routes") },
                        Err(e) => warn!("Go routes connector failed: {e}"),
                    }
                    match crate::connectors::rails_routes::connect(&tdb.conn, &root4) {
                        Ok(n) => if n > 0 { info!("Rails routes connector: {n} routes") },
                        Err(e) => warn!("Rails routes connector failed: {e}"),
                    }
                    match crate::connectors::laravel_routes::connect(&tdb.conn, &root4) {
                        Ok(n) => if n > 0 { info!("Laravel routes connector: {n} routes") },
                        Err(e) => warn!("Laravel routes connector failed: {e}"),
                    }
                    match crate::connectors::nestjs_routes::connect(&tdb.conn, &root4) {
                        Ok(n) => if n > 0 { info!("NestJS routes connector: {n} routes") },
                        Err(e) => warn!("NestJS routes connector failed: {e}"),
                    }
                });
            });
        }
        None => {
            // In-memory database (tests) — run sequentially on the existing connection.
            run_connectors_sequential(db, project_root);
        }
    }
    info!("Connectors completed in {:.2}s", connector_start.elapsed().as_secs_f64());
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

    Ok(stats)
}

// ---------------------------------------------------------------------------
// Parse a single file
// ---------------------------------------------------------------------------

pub(crate) fn parse_file(walked: &WalkedFile) -> Result<ParsedFile> {
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

    let (symbols, refs, routes, db_sets, has_errors) = match walked.language {
        // ---- lang-core: C#, TypeScript, JavaScript ----------------------------
        "csharp" => {
            let r = csharp::extract(&content);
            (r.symbols, r.refs, r.routes, r.db_sets, r.has_errors)
        }
        "typescript" => {
            let r = typescript::extract(&content, false);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        "tsx" => {
            let r = typescript::extract(&content, true);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        "javascript" | "jsx" => {
            let r = crate::parser::extractors::javascript::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        // ---- lang-systems: Rust, Go, C, C++ -----------------------------------
        "rust" => {
            let r = rust::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        "go" => {
            let r = go::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        "c" | "cpp" => {
            let r = crate::parser::extractors::c_lang::extract(&content, walked.language);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        // ---- lang-jvm: Java, Kotlin, Scala ------------------------------------
        "java" => {
            let r = java::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        "kotlin" => {
            let r = crate::parser::extractors::kotlin::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        "scala" => {
            let r = crate::parser::extractors::scala::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        // ---- lang-scripting: Python, Ruby, PHP, Bash, Elixir ------------------
        "python" => {
            let r = python::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        "ruby" => {
            let r = crate::parser::extractors::ruby::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        "php" => {
            let r = crate::parser::extractors::php::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        "shell" => {
            let r = crate::parser::extractors::bash::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        "elixir" => {
            let r = crate::parser::extractors::elixir::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        // ---- lang-mobile: Swift, Dart -----------------------------------------
        "swift" => {
            let r = crate::parser::extractors::swift::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        "dart" => {
            let r = crate::parser::extractors::dart::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        // ---- Generic fallback (all languages with a grammar) ------------------
        _ => match generic::extract(&content, walked.language) {
            Some(r) => (r.symbols, r.refs, vec![], vec![], r.has_errors),
            None => (vec![], vec![], vec![], vec![], false),
        },
    };

    Ok(ParsedFile {
        path: walked.relative_path.clone(),
        language: walked.language.to_string(),
        content_hash: hash,
        size,
        line_count,
        symbols,
        refs,
        routes,
        db_sets,
        content: Some(content),
        has_errors,
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
// Sequential connector fallback (in-memory databases / tests)
// ---------------------------------------------------------------------------

fn run_connectors_sequential(db: &mut Database, project_root: &Path) {
    if let Err(e) = crate::connectors::http_api::connect(db) { warn!("HTTP API connector: {e}"); }
    if let Err(e) = crate::connectors::ef_core::connect(db) { warn!("EF Core connector: {e}"); }
    match crate::connectors::frontend_http::detect_http_calls(&db.conn, project_root) {
        Ok(calls) if !calls.is_empty() => {
            match crate::connectors::frontend_http::match_http_calls_to_routes(&db.conn, &calls) {
                Ok(n) => info!("Frontend HTTP: {} calls, {} matched", calls.len(), n),
                Err(e) => warn!("Frontend HTTP matching: {e}"),
            }
        }
        Err(e) => warn!("Frontend HTTP detection: {e}"),
        _ => {}
    }
    let _ = crate::connectors::dotnet_http_client::connect(&db.conn, project_root).map(|n| if n > 0 { info!(".NET HTTP client: {n} matched") });
    if let Err(e) = crate::connectors::grpc::connect(db) { warn!("gRPC connector: {e}"); }
    match crate::connectors::dotnet_di::detect_di_registrations(&db.conn, project_root) {
        Ok(regs) if !regs.is_empty() => {
            match crate::connectors::dotnet_di::link_di_registrations(&db.conn, &regs) {
                Ok(n) => info!("DI connector: {} registrations, {} edges", regs.len(), n),
                Err(e) => warn!("DI linking: {e}"),
            }
        }
        Err(e) => warn!("DI detection: {e}"),
        _ => {}
    }
    match crate::connectors::dotnet_events::find_integration_events(&db.conn) {
        Ok(events) => {
            match crate::connectors::dotnet_events::find_event_handlers(&db.conn, project_root) {
                Ok(handlers) if !events.is_empty() && !handlers.is_empty() => {
                    match crate::connectors::dotnet_events::link_events_to_handlers(&db.conn, &events, &handlers) {
                        Ok(n) => info!("Events: {} events, {} handlers, {} edges", events.len(), handlers.len(), n),
                        Err(e) => warn!("Event linking: {e}"),
                    }
                }
                Err(e) => warn!("Event handler detection: {e}"),
                _ => {}
            }
        }
        Err(e) => warn!("Integration event detection: {e}"),
    }
    let _ = crate::connectors::tauri_ipc::connect(&db.conn, project_root).map_err(|e| warn!("Tauri IPC: {e}"));
    match crate::connectors::react_patterns::find_zustand_stores(&db.conn, project_root) {
        Ok(stores) => {
            match crate::connectors::react_patterns::find_story_mappings(&db.conn, project_root) {
                Ok(stories) if !stores.is_empty() || !stories.is_empty() => {
                    let _ = crate::connectors::react_patterns::create_react_concepts(&db.conn, &stores, &stories)
                        .map_err(|e| warn!("React concept creation: {e}"));
                }
                Err(e) => warn!("Story mapping: {e}"),
                _ => {}
            }
        }
        Err(e) => warn!("Zustand store detection: {e}"),
    }
    match crate::connectors::spring::find_spring_routes(&db.conn, project_root) {
        Ok(routes) => {
            match crate::connectors::spring::find_spring_services(&db.conn, project_root) {
                Ok(services) if !routes.is_empty() || !services.is_empty() => {
                    let _ = crate::connectors::spring::register_spring_patterns(&db.conn, &routes, &services)
                        .map_err(|e| warn!("Spring patterns: {e}"));
                }
                Err(e) => warn!("Spring services: {e}"),
                _ => {}
            }
        }
        Err(e) => warn!("Spring routes: {e}"),
    }
    let _ = crate::connectors::django::connect(db, project_root).map_err(|e| warn!("Django: {e}"));
    let _ = crate::connectors::graphql::connect(db, project_root).map_err(|e| warn!("GraphQL: {e}"));
    let _ = crate::connectors::message_queue::connect(db, project_root).map_err(|e| warn!("Message queue: {e}"));
    let _ = crate::connectors::electron_ipc::connect(db, project_root).map_err(|e| warn!("Electron IPC: {e}"));
    let _ = crate::connectors::go_routes::connect(&db.conn, project_root).map(|n| if n > 0 { info!("Go routes: {n}") });
    let _ = crate::connectors::rails_routes::connect(&db.conn, project_root).map(|n| if n > 0 { info!("Rails routes: {n}") });
    let _ = crate::connectors::laravel_routes::connect(&db.conn, project_root).map(|n| if n > 0 { info!("Laravel routes: {n}") });
    let _ = crate::connectors::nestjs_routes::connect(&db.conn, project_root).map(|n| if n > 0 { info!("NestJS routes: {n}") });
    let _ = crate::connectors::fastapi_routes::connect(&db.conn, project_root).map(|n| if n > 0 { info!("FastAPI routes: {n}") });
    let _ = crate::connectors::spring_di::connect(&db.conn, project_root).map(|n| if n > 0 { info!("Spring DI: {n}") });
    let _ = crate::connectors::angular_di::connect(&db.conn, project_root).map(|n| if n > 0 { info!("Angular DI: {n}") });
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
