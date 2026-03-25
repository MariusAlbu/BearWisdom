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
    // full_index is a complete rebuild. Drop and recreate core tables
    // rather than DELETE (which is slow on large tables due to index updates).
    {
        let count: i64 = db.conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0)).unwrap_or(0);
        if count > 0 {
            info!("Clearing {} existing files from index for full rebuild", count);
            // Drop dependent tables first, then recreate via the schema init.
            // The schema init in Database::open already ensures tables exist,
            // so we just need to clear data. Use a transaction for atomicity.
            // Clear vec_chunks first (virtual table — not covered by CASCADE).
            if crate::search::vector_store::vec_table_exists(&db.conn) {
                let _ = db.conn.execute_batch("DELETE FROM vec_chunks");
            }
            let _ = db.conn.execute_batch(
                "PRAGMA foreign_keys = OFF;
                 DELETE FROM edges;
                 DELETE FROM imports;
                 DELETE FROM unresolved_refs;
                 DELETE FROM symbols;
                 DELETE FROM files;
                 PRAGMA foreign_keys = ON;"
            );
            info!("Index tables cleared");
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
    let (edge_count, unresolved_count) =
        resolve::resolve_and_write(db, &parsed, &symbol_id_map)
            .context("Failed to resolve references")?;
    info!(
        "Wrote {} edges, {} unresolved references",
        edge_count, unresolved_count
    );
    emit("resolving", 1.0, Some(&format!("{} edges resolved", edge_count)));

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

    // --- Step 7: Connectors ---
    emit("connectors", 0.0, Some("Running connectors"));
    crate::connectors::http_api::connect(db)
        .context("HTTP API connector failed")?;
    crate::connectors::ef_core::connect(db)
        .context("EF Core connector failed")?;

    // --- Step 7b: Frontend HTTP connector ---
    match crate::connectors::frontend_http::detect_http_calls(&db.conn, project_root) {
        Ok(http_calls) => {
            if !http_calls.is_empty() {
                match crate::connectors::frontend_http::match_http_calls_to_routes(
                    &db.conn,
                    &http_calls,
                ) {
                    Ok(matched) => info!(
                        "Frontend HTTP: {} calls detected, {} matched to routes",
                        http_calls.len(),
                        matched
                    ),
                    Err(e) => warn!("Frontend HTTP route matching failed: {e}"),
                }
            }
        }
        Err(e) => warn!("Frontend HTTP detection failed: {e}"),
    }

    // --- Step 7c: gRPC connector ---
    crate::connectors::grpc::connect(db)
        .context("gRPC connector failed")?;

    // --- Step 7d: .NET DI connector ---
    match crate::connectors::dotnet_di::detect_di_registrations(&db.conn, project_root) {
        Ok(registrations) => {
            if !registrations.is_empty() {
                match crate::connectors::dotnet_di::link_di_registrations(
                    &db.conn,
                    &registrations,
                ) {
                    Ok(linked) => info!(
                        "DI connector: {} registrations detected, {} edges created",
                        registrations.len(),
                        linked
                    ),
                    Err(e) => warn!("DI registration linking failed: {e}"),
                }
            }
        }
        Err(e) => warn!("DI registration detection failed: {e}"),
    }

    // --- Step 7e: .NET integration events connector ---
    match crate::connectors::dotnet_events::find_integration_events(&db.conn) {
        Ok(events) => {
            match crate::connectors::dotnet_events::find_event_handlers(
                &db.conn,
                project_root,
            ) {
                Ok(handlers) => {
                    if !events.is_empty() && !handlers.is_empty() {
                        match crate::connectors::dotnet_events::link_events_to_handlers(
                            &db.conn,
                            &events,
                            &handlers,
                        ) {
                            Ok(linked) => info!(
                                "Events connector: {} events, {} handlers, {} edges",
                                events.len(),
                                handlers.len(),
                                linked
                            ),
                            Err(e) => warn!("Event linking failed: {e}"),
                        }
                    }
                }
                Err(e) => warn!("Event handler detection failed: {e}"),
            }
        }
        Err(e) => warn!("Integration event detection failed: {e}"),
    }

    // --- Step 7f: Tauri IPC connector ---
    match crate::connectors::tauri_ipc::connect(&db.conn, project_root) {
        Ok(()) => info!("Tauri IPC connector complete"),
        Err(e) => warn!("Tauri IPC connector failed: {e}"),
    }

    // --- Step 7g: React patterns connector ---
    match crate::connectors::react_patterns::find_zustand_stores(&db.conn, project_root) {
        Ok(stores) => {
            match crate::connectors::react_patterns::find_story_mappings(
                &db.conn,
                project_root,
            ) {
                Ok(stories) => {
                    if !stores.is_empty() || !stories.is_empty() {
                        match crate::connectors::react_patterns::create_react_concepts(
                            &db.conn,
                            &stores,
                            &stories,
                        ) {
                            Ok(()) => info!(
                                "React patterns: {} stores, {} stories processed",
                                stores.len(),
                                stories.len()
                            ),
                            Err(e) => warn!("React concept creation failed: {e}"),
                        }
                    }
                }
                Err(e) => warn!("Story mapping detection failed: {e}"),
            }
        }
        Err(e) => warn!("Zustand store detection failed: {e}"),
    }

    // --- Step 7h: Spring connector ---
    match crate::connectors::spring::find_spring_routes(&db.conn, project_root) {
        Ok(routes) => {
            match crate::connectors::spring::find_spring_services(&db.conn, project_root) {
                Ok(services) => {
                    if !routes.is_empty() || !services.is_empty() {
                        match crate::connectors::spring::register_spring_patterns(
                            &db.conn,
                            &routes,
                            &services,
                        ) {
                            Ok(()) => info!(
                                "Spring connector: {} routes, {} services processed",
                                routes.len(),
                                services.len()
                            ),
                            Err(e) => warn!("Spring pattern registration failed: {e}"),
                        }
                    }
                }
                Err(e) => warn!("Spring service detection failed: {e}"),
            }
        }
        Err(e) => warn!("Spring route detection failed: {e}"),
    }

    // --- Step 7i: Django connector ---
    match crate::connectors::django::connect(db, project_root) {
        Ok(()) => info!("Django connector complete"),
        Err(e) => warn!("Django connector failed: {e}"),
    }

    // --- Step 7j: GraphQL connector ---
    match crate::connectors::graphql::connect(db, project_root) {
        Ok(()) => info!("GraphQL connector complete"),
        Err(e) => warn!("GraphQL connector failed: {e}"),
    }

    // --- Step 7k: Message queue connector ---
    match crate::connectors::message_queue::connect(db, project_root) {
        Ok(()) => info!("Message queue connector complete"),
        Err(e) => warn!("Message queue connector failed: {e}"),
    }

    // --- Step 7l: Electron IPC connector ---
    match crate::connectors::electron_ipc::connect(db, project_root) {
        Ok(()) => info!("Electron IPC connector complete"),
        Err(e) => warn!("Electron IPC connector failed: {e}"),
    }

    emit("connectors", 1.0, None);

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
        // ---- Dedicated extractors (full symbol + ref + route extraction) ----
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
        "rust" => {
            let r = rust::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        "python" => {
            let r = python::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        "go" => {
            let r = go::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        "java" => {
            let r = java::extract(&content);
            (r.symbols, r.refs, vec![], vec![], r.has_errors)
        }
        // ---- Generic grammar-based extraction (all other supported languages)
        // Languages where a grammar exists but no dedicated extractor has been
        // written yet fall through to the generic DFS walker.  When a dedicated
        // extractor is added, add a match arm above this block.
        _ => match generic::extract(&content, walked.language) {
            Some(r) => (r.symbols, r.refs, vec![], vec![], r.has_errors),
            // No grammar available for this language — index file with zero symbols.
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

    for pf in parsed {
        // Upsert the file row (delete existing symbols via CASCADE, then re-insert).
        tx.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET
               hash = excluded.hash,
               language = excluded.language,
               last_indexed = excluded.last_indexed",
            rusqlite::params![pf.path, pf.content_hash, pf.language, now],
        ).with_context(|| format!("Failed to upsert file {}", pf.path))?;

        // If it was an UPDATE the last_insert_rowid() returns 0 on some platforms.
        // Re-fetch by path to be safe.
        let file_id: i64 = tx.query_row(
            "SELECT id FROM files WHERE path = ?1",
            [&pf.path],
            |r| r.get(0),
        ).with_context(|| format!("Failed to get file_id for {}", pf.path))?;

        file_id_map.insert(pf.path.clone(), file_id);

        // Delete existing symbols for this file so we can re-insert cleanly.
        // (The ON CONFLICT above updates the file row but doesn't cascade-delete symbols.)
        tx.execute("DELETE FROM symbols WHERE file_id = ?1", [file_id])?;

        // Delete existing imports for this file (not cascaded by symbols delete).
        tx.execute("DELETE FROM imports WHERE file_id = ?1", [file_id])?;

        // Insert all symbols for this file.
        for sym in &pf.symbols {
            tx.execute(
                "INSERT INTO symbols
                   (file_id, name, qualified_name, kind, line, col,
                    end_line, end_col, scope_path, signature, doc_comment, visibility)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
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
                ],
            ).with_context(|| format!("Failed to insert symbol {} in {}", sym.qualified_name, pf.path))?;

            let sym_id = tx.last_insert_rowid();
            symbol_id_map.insert((pf.path.clone(), sym.qualified_name.clone()), sym_id);
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
            tx.execute(
                "INSERT INTO imports (file_id, imported_name, module_path, alias, line)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    file_id,
                    imported_name,
                    module_path,
                    Option::<&str>::None, // alias extraction not yet implemented
                    r.line,
                ],
            ).with_context(|| format!("Failed to insert import '{}' in {}", imported_name, pf.path))?;
        }
    }

    tx.commit().context("Failed to commit file/symbol transaction")?;
    Ok((file_id_map, symbol_id_map))
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
    let route_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM routes", [], |r| r.get(0))?;
    let db_mapping_count: u32 =
        conn.query_row("SELECT COUNT(*) FROM db_mappings", [], |r| r.get(0))?;

    Ok(IndexStats {
        file_count,
        symbol_count,
        edge_count,
        unresolved_ref_count,
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
mod tests {
    use super::*;
    use crate::db::Database;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn index_simple_csharp_project() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("Foo.cs"),
            r#"
namespace App {
    public class FooService {
        public void DoSomething() {}
    }
}
"#,
        ).unwrap();

        let mut db = Database::open_in_memory().unwrap();
        let stats = full_index(&mut db, dir.path(), None, None).unwrap();

        assert!(stats.file_count >= 1, "No files indexed");
        assert!(stats.symbol_count >= 2, "Expected at least FooService + DoSomething");
    }

    #[test]
    fn index_produces_qualified_names() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("Api.cs"),
            "namespace Catalog { class CatalogApi { void List() {} } }",
        ).unwrap();

        let mut db = Database::open_in_memory().unwrap();
        full_index(&mut db, dir.path(), None, None).unwrap();

        let qname: String = db.conn.query_row(
            "SELECT qualified_name FROM symbols WHERE name = 'List'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(qname, "Catalog.CatalogApi.List");
    }

    #[test]
    fn index_empty_directory_produces_zero_stats() {
        let dir = TempDir::new().unwrap();
        let mut db = Database::open_in_memory().unwrap();
        let stats = full_index(&mut db, dir.path(), None, None).unwrap();
        assert_eq!(stats.file_count, 0);
        assert_eq!(stats.symbol_count, 0);
    }
}
