// =============================================================================
// indexer/write.rs  —  shared write pipeline
//
// Single source of truth for writing parsed files to the database.
// Both full and incremental indexers call these functions — no more
// duplicated SQL or diverging statement preparation strategies.
// =============================================================================

use crate::db::Database;
use crate::types::ParsedFile;
use anyhow::{Context, Result};
use rusqlite::types::Value;
use std::collections::HashMap;
use tracing::{debug, warn};

// Batching constants. SQLite's `SQLITE_MAX_VARIABLE_NUMBER` defaults to
// 32766 on modern builds; 128 rows × 14 vars = 1792 variables, well
// inside any realistic limit. Row counts are chosen so almost every file
// fits in one batch (median C# file has < 128 symbols) — a bigger batch
// would only save us on outlier files and risk hitting the variable
// cap on pathological generated code.
const SYMBOL_COLS: usize = 14;
const SYMBOL_BATCH_ROWS: usize = 128;
const IMPORT_COLS: usize = 5;
const IMPORT_BATCH_ROWS: usize = 256;

/// Maps relative_path → SQLite file row ID.
pub type FileIdMap = HashMap<String, i64>;

/// Maps (relative_path, qualified_name) → SQLite symbol row ID.
pub type SymbolIdMap = HashMap<(String, String), i64>;

fn symbol_insert_sql(rows: usize) -> String {
    // Pre-sized: 14 vars per row, one tuple plus a separator of 3 chars,
    // plus the header/footer. 64 is a comfortable overshoot.
    let mut sql = String::with_capacity(256 + rows * 64);
    sql.push_str(
        "INSERT INTO symbols \
         (file_id, name, qualified_name, kind, line, col, end_line, end_col, \
          scope_path, signature, doc_comment, visibility, origin, origin_language) \
         VALUES ",
    );
    for i in 0..rows {
        if i > 0 { sql.push(','); }
        sql.push_str("(?,?,?,?,?,?,?,?,?,?,?,?,?,?)");
    }
    sql.push_str(" RETURNING id");
    sql
}

fn import_insert_sql(rows: usize) -> String {
    let mut sql = String::with_capacity(128 + rows * 24);
    sql.push_str(
        "INSERT INTO imports (file_id, imported_name, module_path, alias, line) VALUES ",
    );
    for i in 0..rows {
        if i > 0 { sql.push(','); }
        sql.push_str("(?,?,?,?,?)");
    }
    sql
}

fn push_symbol_params(
    params: &mut Vec<Value>,
    file_id: i64,
    pf: &ParsedFile,
    global_idx: usize,
    origin: &str,
) {
    let sym = &pf.symbols[global_idx];
    let origin_language: Option<&str> = pf
        .symbol_origin_languages
        .get(global_idx)
        .and_then(|o| o.as_deref());
    params.push(Value::Integer(file_id));
    params.push(Value::Text(sym.name.clone()));
    params.push(Value::Text(sym.qualified_name.clone()));
    params.push(Value::Text(sym.kind.as_str().to_string()));
    params.push(Value::Integer(sym.start_line as i64));
    params.push(Value::Integer(sym.start_col as i64));
    params.push(Value::Integer(sym.end_line as i64));
    params.push(Value::Integer(sym.end_col as i64));
    params.push(match &sym.scope_path {
        Some(s) => Value::Text(s.clone()),
        None => Value::Null,
    });
    params.push(match &sym.signature {
        Some(s) => Value::Text(s.clone()),
        None => Value::Null,
    });
    params.push(match &sym.doc_comment {
        Some(s) => Value::Text(s.clone()),
        None => Value::Null,
    });
    params.push(match sym.visibility {
        Some(v) => Value::Text(v.as_str().to_string()),
        None => Value::Null,
    });
    params.push(Value::Text(origin.to_string()));
    params.push(match origin_language {
        Some(s) => Value::Text(s.to_string()),
        None => Value::Null,
    });
}

/// Batched symbol insert. Replaces the per-row loop: for a file with 400
/// symbols, the old path executed 400 separate `INSERT … RETURNING id`
/// statements; this path runs 4 chunked `INSERT … VALUES (…),(…),…
/// RETURNING id` statements, cutting the rusqlite round-trip count by
/// ~100×. Preserves the SymbolIdMap ordering the rest of the pipeline
/// assumes: SQLite's RETURNING returns rows in VALUES order.
fn insert_symbols_batched(
    tx: &rusqlite::Transaction<'_>,
    file_id: i64,
    pf: &ParsedFile,
    origin: &str,
    symbol_id_map: &mut SymbolIdMap,
) -> Result<()> {
    if pf.symbols.is_empty() { return Ok(()); }

    let total = pf.symbols.len();
    let mut start = 0;
    while start < total {
        let end = (start + SYMBOL_BATCH_ROWS).min(total);
        let rows = end - start;
        let sql = symbol_insert_sql(rows);
        let mut params: Vec<Value> = Vec::with_capacity(rows * SYMBOL_COLS);
        for i in start..end {
            push_symbol_params(&mut params, file_id, pf, i, origin);
        }

        // prepare_cached hits when the chunk is exactly SYMBOL_BATCH_ROWS
        // (true for every non-tail chunk of a large file). The tail chunk
        // is a one-shot prepare, negligible.
        let mut stmt = tx
            .prepare_cached(&sql)
            .context("Failed to prepare batched symbol insert")?;
        let ids: Vec<i64> = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |r| r.get(0))
            .context("Failed to execute batched symbol insert")?
            .collect::<std::result::Result<_, _>>()
            .context("Failed to collect RETURNING ids")?;

        if ids.len() != rows {
            anyhow::bail!(
                "RETURNING id count mismatch: expected {} rows, got {} for {}",
                rows, ids.len(), pf.path,
            );
        }
        for (i, sym_id) in (start..end).zip(ids.iter()) {
            let sym = &pf.symbols[i];
            symbol_id_map.insert((pf.path.clone(), sym.qualified_name.clone()), *sym_id);
        }
        start = end;
    }
    Ok(())
}

/// Batched import insert. Filters non-Imports refs, then chunks.
fn insert_imports_batched(
    tx: &rusqlite::Transaction<'_>,
    file_id: i64,
    pf: &ParsedFile,
) -> Result<()> {
    let imports: Vec<&crate::types::ExtractedRef> = pf
        .refs
        .iter()
        .filter(|r| r.kind == crate::types::EdgeKind::Imports)
        .collect();
    if imports.is_empty() { return Ok(()); }

    let mut start = 0;
    while start < imports.len() {
        let end = (start + IMPORT_BATCH_ROWS).min(imports.len());
        let rows = end - start;
        let sql = import_insert_sql(rows);
        let mut params: Vec<Value> = Vec::with_capacity(rows * IMPORT_COLS);
        for r in &imports[start..end] {
            params.push(Value::Integer(file_id));
            params.push(Value::Text(r.target_name.clone()));
            params.push(match r.module.as_deref() {
                Some(s) => Value::Text(s.to_string()),
                None => Value::Null,
            });
            params.push(Value::Null); // alias — always null in extract
            params.push(Value::Integer(r.line as i64));
        }

        tx.prepare_cached(&sql)
            .context("Failed to prepare batched import insert")?
            .execute(rusqlite::params_from_iter(params.iter()))
            .context("Failed to execute batched import insert")?;
        start = end;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Core write: files + symbols + imports + routes
// ---------------------------------------------------------------------------

/// Write parsed files and their symbols in a single transaction.
///
/// Returns two maps used by the resolver:
///   - `FileIdMap`: relative path → file row ID
///   - `SymbolIdMap`: (relative_path, qualified_name) → symbol row ID
///
/// All statements use `prepare_cached` for optimal performance.
pub fn write_parsed_files(
    db: &Database,
    parsed: &[ParsedFile],
) -> Result<(FileIdMap, SymbolIdMap)> {
    // Incremental entry point: symbols/imports for these files may exist
    // from a prior index, so the per-file DELETE step is load-bearing.
    write_parsed_files_with_origin_impl(db, parsed, "internal", /*is_full*/ false)
}

/// Write a single `ParsedFile` inside an existing transaction and return its
/// assigned `file_id`. Appends rows into `symbol_id_map` for every symbol.
///
/// Used by the streaming parse pipeline in `full.rs` so files can be
/// persisted one at a time as parser workers produce them, instead of
/// holding every ParsedFile in memory until a single batched write.
///
/// Caller is responsible for transaction lifecycle (begin + commit) and
/// for invalidating any query cache once all writes are done.
pub fn write_one_parsed_file(
    tx: &rusqlite::Transaction<'_>,
    pf: &ParsedFile,
    origin: &str,
    now: i64,
    symbol_id_map: &mut SymbolIdMap,
    is_full: bool,
) -> Result<i64> {
    // Upsert file row and capture the assigned id via RETURNING.
    let file_id: i64 = tx
        .prepare_cached(
            "INSERT INTO files (path, hash, language, last_indexed, mtime, size, package_id, origin)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(path) DO UPDATE SET
               hash = excluded.hash,
               language = excluded.language,
               last_indexed = excluded.last_indexed,
               mtime = excluded.mtime,
               size = excluded.size,
               package_id = excluded.package_id,
               origin = excluded.origin
             RETURNING id",
        )
        .context("Failed to prepare file upsert")?
        .query_row(
            rusqlite::params![pf.path, pf.content_hash, pf.language, now, pf.mtime, pf.size as i64, pf.package_id, origin],
            |r| r.get(0),
        )
        .with_context(|| format!("Failed to upsert file {}", pf.path))?;

    // On a full index the symbols / imports tables were just DROP+CREATE'd
    // in `full.rs`, so these per-file DELETEs are no-ops — but a no-op
    // DELETE is still a round-trip through rusqlite + SQLite's statement
    // executor. Across ~1M files this is tens of seconds of wall-clock.
    // Incremental callers still need the DELETE to clear stale rows.
    if !is_full {
        tx.prepare_cached("DELETE FROM symbols WHERE file_id = ?1")
            .context("Failed to prepare symbol delete")?
            .execute([file_id])?;
        tx.prepare_cached("DELETE FROM imports WHERE file_id = ?1")
            .context("Failed to prepare import delete")?
            .execute([file_id])?;
    }

    insert_symbols_batched(tx, file_id, pf, origin, symbol_id_map)?;

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
               (file_id, symbol_id, http_method, route_template, resolved_route, line)
             VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
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

    insert_imports_batched(tx, file_id, pf)?;

    Ok(file_id)
}

/// Origin-aware variant. Callers that index external dependency sources
/// (Go module cache, node_modules, site-packages, etc.) pass "external" so
/// the rows can be partitioned from project code in user-facing queries.
pub fn write_parsed_files_with_origin(
    db: &Database,
    parsed: &[ParsedFile],
    origin: &str,
) -> Result<(FileIdMap, SymbolIdMap)> {
    // Default to the full-index fast path (tables are fresh after
    // DROP+CREATE). Call sites that re-write over existing rows use the
    // `_incremental` variant, which keeps the per-file DELETE cleanup.
    write_parsed_files_with_origin_impl(db, parsed, origin, /*is_full*/ true)
}

/// Incremental-safe variant: keeps per-file DELETE from symbols/imports so
/// stale rows are removed when a file is re-indexed.
pub fn write_parsed_files_with_origin_incremental(
    db: &Database,
    parsed: &[ParsedFile],
    origin: &str,
) -> Result<(FileIdMap, SymbolIdMap)> {
    write_parsed_files_with_origin_impl(db, parsed, origin, /*is_full*/ false)
}

fn write_parsed_files_with_origin_impl(
    db: &Database,
    parsed: &[ParsedFile],
    origin: &str,
    is_full: bool,
) -> Result<(FileIdMap, SymbolIdMap)> {
    let conn = db.conn();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin transaction")?;

    let mut file_id_map: FileIdMap = HashMap::new();
    let mut symbol_id_map: SymbolIdMap = HashMap::new();

    for pf in parsed {
        // Upsert file row and capture the assigned id via RETURNING.
        let file_id: i64 = tx
            .prepare_cached(
                "INSERT INTO files (path, hash, language, last_indexed, mtime, size, package_id, origin)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(path) DO UPDATE SET
                   hash = excluded.hash,
                   language = excluded.language,
                   last_indexed = excluded.last_indexed,
                   mtime = excluded.mtime,
                   size = excluded.size,
                   package_id = excluded.package_id,
                   origin = excluded.origin
                 RETURNING id",
            )
            .context("Failed to prepare file upsert")?
            .query_row(
                rusqlite::params![pf.path, pf.content_hash, pf.language, now, pf.mtime, pf.size as i64, pf.package_id, origin],
                |r| r.get(0),
            )
            .with_context(|| format!("Failed to upsert file {}", pf.path))?;

        file_id_map.insert(pf.path.clone(), file_id);

        // On a full index the tables were just DROP+CREATE'd in full.rs so
        // these per-file DELETEs are no-ops. Skipping them saves ~2 SQL
        // round-trips per file (tens of seconds on 500k+ files).
        if !is_full {
            // Delete existing symbols (ON CONFLICT upsert doesn't cascade-delete).
            tx.prepare_cached("DELETE FROM symbols WHERE file_id = ?1")
                .context("Failed to prepare symbol delete")?
                .execute([file_id])?;

            // Delete existing imports (not cascaded by symbols delete).
            tx.prepare_cached("DELETE FROM imports WHERE file_id = ?1")
                .context("Failed to prepare import delete")?
                .execute([file_id])?;
        }

        // Sub-extracted symbols carry their own origin language (e.g. TS
        // inside a .vue file). Host-extracted symbols use the file's
        // language — represented as NULL in the column for storage
        // efficiency and queryability ("WHERE origin_language IS NOT NULL"
        // yields only spliced multi-language symbols).
        insert_symbols_batched(&tx, file_id, pf, origin, &mut symbol_id_map)?;

        // Insert route records (ASP.NET [HttpGet], [Route], etc.).
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

            // `resolved_route` defaults to `route_template` at extract time —
            // connectors that know about controller-prefix / mount-path joining
            // later overwrite the resolved path with the full concatenation.
            // Previously a post-parse `UPDATE routes SET resolved_route =
            // route_template WHERE resolved_route IS NULL` did this; writing
            // it inline here drops one SQL round-trip per full reindex.
            tx.prepare_cached(
                "INSERT OR IGNORE INTO routes
                   (file_id, symbol_id, http_method, route_template, resolved_route, line)
                 VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
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

        // Insert import records.
        insert_imports_batched(&tx, file_id, pf)?;
    }

    tx.commit()
        .context("Failed to commit file/symbol transaction")?;

    // Invalidate query caches — symbols changed.
    if let Some(ref cache) = db.query_cache {
        cache.invalidate_all();
    }

    Ok((file_id_map, symbol_id_map))
}

// ---------------------------------------------------------------------------
// FTS content indexing
// ---------------------------------------------------------------------------

/// Update the FTS5 trigram content index for parsed files.
///
/// For incremental: deletes old entries for files being re-indexed,
/// then inserts current content.
pub fn update_fts_content(
    db: &Database,
    parsed: &[ParsedFile],
    file_id_map: &FileIdMap,
) -> Result<u32> {
    let conn = db.conn();
    let mut count = 0u32;

    // For incremental updates, clean up old FTS entries first.
    // For full index after DROP+CREATE this is a no-op (table is empty).
    for pf in parsed {
        if let Some(&file_id) = file_id_map.get(&pf.path) {
            let _ = conn.execute("DELETE FROM fts_content WHERE rowid = ?1", [file_id]);
        }
    }

    // Batch-insert using the content_index module when available.
    let content_entries: Vec<(i64, &str, &str)> = parsed
        .iter()
        .filter_map(|pf| {
            let file_id = file_id_map.get(&pf.path)?;
            let content = pf.content.as_deref()?;
            Some((*file_id, pf.path.as_str(), content))
        })
        .collect();

    match crate::search::content_index::batch_index_content(conn, &content_entries) {
        Ok(n) => count = n as u32,
        Err(e) => warn!("FTS5 content indexing failed: {e}"),
    }

    Ok(count)
}

// ---------------------------------------------------------------------------
// Code chunking (for embedding/vector search)
// ---------------------------------------------------------------------------

/// Chunk parsed files for embedding and store in `code_chunks`.
///
/// When `is_full` is true (full index after DROP+CREATE), uses the bulk insert
/// path: computes all chunks in memory, batch-inserts in one transaction, skips
/// dedup entirely.  This avoids 50k individual queries on an empty table.
///
/// When `is_full` is false (incremental), uses per-file hash-based dedup to
/// preserve existing vectors for unchanged chunks.
pub fn update_chunks(
    db: &Database,
    parsed: &[ParsedFile],
    file_id_map: &FileIdMap,
    is_full: bool,
) -> Result<u32> {
    let conn = db.conn();

    if is_full {
        // Bulk path: no dedup, no cleanup, one transaction.
        let files: Vec<(i64, &str)> = parsed
            .iter()
            .filter_map(|pf| {
                let file_id = file_id_map.get(&pf.path)?;
                let content = pf.content.as_deref()?;
                Some((*file_id, content))
            })
            .collect();

        match crate::search::chunker::bulk_chunk_and_store(conn, &files) {
            Ok(n) => Ok(n),
            Err(e) => {
                warn!("Bulk chunking failed: {e}");
                Ok(0)
            }
        }
    } else {
        // Incremental path: per-file dedup preserves existing vectors.
        let mut total = 0u32;
        for pf in parsed {
            if let (Some(&file_id), Some(content)) =
                (file_id_map.get(&pf.path), pf.content.as_deref())
            {
                let _ = crate::search::vector_store::delete_file_vectors(conn, file_id);
                let _ = conn.execute("DELETE FROM code_chunks WHERE file_id = ?1", [file_id]);

                match crate::search::chunker::chunk_and_store(conn, file_id, content) {
                    Ok(n) => total += n,
                    Err(e) => debug!("Failed to chunk {}: {e}", pf.path),
                }
            }
        }
        Ok(total)
    }
}

// ---------------------------------------------------------------------------
// File deletion
// ---------------------------------------------------------------------------

/// Delete files from the index by relative path.
///
/// Handles CASCADE-covered tables (symbols, edges, etc.) via the FK
/// constraint, plus virtual tables (vec_chunks, fts_content, flow_edges)
/// that require manual cleanup.
///
/// All per-file DELETE statements run inside a single transaction so the
/// database is never left in a partially-deleted state if the process is
/// interrupted mid-batch.
pub fn delete_files(db: &Database, paths: &[String]) -> Result<Vec<(i64, String)>> {
    let conn = db.conn();
    let mut deleted = Vec::new();

    if paths.is_empty() {
        return Ok(deleted);
    }

    // Resolve file IDs outside the transaction (read-only).
    let mut file_ids: Vec<(i64, &String)> = Vec::with_capacity(paths.len());
    for rel_path in paths {
        if let Ok(file_id) = conn.query_row(
            "SELECT id FROM files WHERE path = ?1",
            [rel_path.as_str()],
            |r| r.get::<_, i64>(0),
        ) {
            file_ids.push((file_id, rel_path));
        }
    }

    if file_ids.is_empty() {
        return Ok(deleted);
    }

    // Virtual-table cleanup must happen before the transaction DELETEs the
    // rows that the virtual tables reference.  sqlite-vec is not transactional
    // in the same sense, so we clean it up first while the rows still exist.
    for (file_id, _) in &file_ids {
        let _ = crate::search::vector_store::delete_file_vectors(conn, *file_id);
    }

    // Wrap all DELETE statements in one transaction.
    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin delete transaction")?;

    for (file_id, rel_path) in &file_ids {
        // CASCADE handles symbols, edges, imports, routes, code_chunks,
        // connection_points, etc.
        tx.execute("DELETE FROM files WHERE id = ?1", [file_id])?;

        // Manual cleanup for tables without FK to files.
        let _ = tx.execute("DELETE FROM fts_content WHERE rowid = ?1", [file_id]);
        let _ = tx.execute(
            "DELETE FROM flow_edges WHERE source_file_id = ?1 OR target_file_id = ?1",
            [file_id],
        );

        deleted.push((*file_id, (*rel_path).clone()));
        debug!("Deleted file from index: {rel_path}");
    }

    tx.commit().context("Failed to commit delete transaction")?;

    Ok(deleted)
}

// ---------------------------------------------------------------------------
// Package write/detect
// ---------------------------------------------------------------------------

/// Write detected packages to the `packages` table and return them with IDs assigned.
///
/// Existing packages (matched by path) are updated; new ones are inserted.
/// Returns the full list with `id` populated.
pub fn write_packages(
    db: &Database,
    packages: &[crate::types::PackageInfo],
) -> Result<Vec<crate::types::PackageInfo>> {
    let conn = db.conn();
    let mut result = Vec::with_capacity(packages.len());

    for pkg in packages {
        // Composite identity is (path, kind); kind is NOT NULL in the new
        // schema. Old detectors that left kind unset get bucketed as
        // 'unknown' so the conflict target stays well-defined.
        let kind_value = pkg.kind.clone().unwrap_or_else(|| "unknown".to_string());
        let id: i64 = conn
            .prepare_cached(
                "INSERT INTO packages (name, path, kind, manifest, declared_name)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(path, kind) DO UPDATE SET
                   name = excluded.name,
                   manifest = excluded.manifest,
                   declared_name = excluded.declared_name
                 RETURNING id",
            )?
            .query_row(
                rusqlite::params![
                    pkg.name,
                    pkg.path,
                    kind_value,
                    pkg.manifest,
                    pkg.declared_name,
                ],
                |r| r.get(0),
            )
            .with_context(|| format!("Failed to upsert package {} ({})", pkg.name, kind_value))?;

        result.push(crate::types::PackageInfo {
            id: Some(id),
            name: pkg.name.clone(),
            path: pkg.path.clone(),
            kind: Some(kind_value),
            manifest: pkg.manifest.clone(),
            declared_name: pkg.declared_name.clone(),
        });
    }

    // Remove packages that are no longer detected. Composite key means we
    // delete by (path, kind) tuples — a path may legitimately be re-used
    // across ecosystems (Tauri root: cargo + npm).
    if !packages.is_empty() {
        let kind_buf: Vec<String> = packages
            .iter()
            .map(|p| p.kind.clone().unwrap_or_else(|| "unknown".to_string()))
            .collect();
        let tuples: String = (1..=packages.len())
            .map(|i| format!("(?{}, ?{})", i * 2 - 1, i * 2))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "DELETE FROM packages WHERE (path, kind) NOT IN (VALUES {tuples})"
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let mut params: Vec<&dyn rusqlite::types::ToSql> = Vec::with_capacity(packages.len() * 2);
        for (pkg, kind) in packages.iter().zip(kind_buf.iter()) {
            params.push(&pkg.path as &dyn rusqlite::types::ToSql);
            params.push(kind as &dyn rusqlite::types::ToSql);
        }
        stmt.execute(params.as_slice())?;
    } else {
        // No packages detected this run — clear all stale rows.
        conn.execute("DELETE FROM packages", [])?;
    }

    Ok(result)
}

/// Assign `package_id` to each parsed file based on longest path-prefix match.
pub fn assign_package_ids(
    parsed: &mut [crate::types::ParsedFile],
    packages: &[crate::types::PackageInfo],
) {
    if packages.is_empty() {
        return;
    }
    // Sort packages by path length descending for longest-prefix-first matching.
    let mut sorted: Vec<&crate::types::PackageInfo> = packages.iter().collect();
    sorted.sort_by(|a, b| b.path.len().cmp(&a.path.len()));

    for pf in parsed.iter_mut() {
        for pkg in &sorted {
            // Normalize separators for comparison.
            let file_path = pf.path.replace('\\', "/");
            let pkg_path = pkg.path.replace('\\', "/");
            if pkg_path.is_empty() {
                // Root package: any file inside the project belongs to it.
                // Sort order (length desc) ensures this only fires when no
                // proper-prefix package matched first. Mirrors
                // `package_id_for_path` in `full.rs`.
                pf.package_id = pkg.id;
                break;
            }
            if file_path.starts_with(&pkg_path)
                && (file_path.len() == pkg_path.len()
                    || file_path.as_bytes()[pkg_path.len()] == b'/')
            {
                pf.package_id = pkg.id;
                break;
            }
        }
    }
}

/// M3: Write per-package dependency declarations to `package_deps`.
///
/// `entries` is a list of `(package_id, ecosystem, dep_name, version, kind)`
/// rows derived from each workspace package's manifest data during
/// `parse_external_sources`. The write is an upsert on the composite
/// primary key `(package_id, ecosystem, dep_name)` — re-running a full
/// index replaces any stale version/kind without leaving duplicates.
///
/// Callers should `DELETE FROM package_deps` first on incremental paths
/// that discover a shrunk manifest; full index drops + recreates the
/// table so no explicit clear is needed there.
pub fn write_package_deps(
    db: &Database,
    entries: &[(i64, &str, String, Option<String>, &'static str)],
) -> Result<usize> {
    if entries.is_empty() {
        return Ok(0);
    }
    let conn = db.conn();
    let mut stmt = conn.prepare_cached(
        "INSERT INTO package_deps (package_id, ecosystem, dep_name, version, kind)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(package_id, ecosystem, dep_name) DO UPDATE SET
           version = excluded.version,
           kind    = excluded.kind",
    )?;
    let mut written = 0usize;
    for (pkg_id, ecosystem, dep_name, version, kind) in entries {
        stmt.execute(rusqlite::params![pkg_id, ecosystem, dep_name, version, kind])?;
        written += 1;
    }
    Ok(written)
}

// ---------------------------------------------------------------------------
// Package loading (for incremental package_id assignment)
// ---------------------------------------------------------------------------

/// Load all packages from the `packages` table.
///
/// Used during incremental indexing to assign `package_id` to newly parsed
/// files without re-running full package detection.
pub fn load_packages_from_db(db: &Database) -> Result<Vec<crate::types::PackageInfo>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, name, path, kind, manifest, declared_name FROM packages",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(crate::types::PackageInfo {
            id: Some(r.get::<_, i64>(0)?),
            name: r.get::<_, String>(1)?,
            path: r.get::<_, String>(2)?,
            kind: r.get::<_, Option<String>>(3)?,
            manifest: r.get::<_, Option<String>>(4)?,
            declared_name: r.get::<_, Option<String>>(5)?,
        })
    })?;
    let mut packages = Vec::new();
    for row in rows {
        packages.push(row?);
    }
    Ok(packages)
}

// ---------------------------------------------------------------------------
// Symbol ID loading (for incremental resolution)
// ---------------------------------------------------------------------------

/// Load the full symbol_id_map from the database.
///
/// Used during incremental resolution so the resolver can see symbols from
/// unchanged files (not just the ones in the current parse batch).
pub fn load_symbol_id_map(db: &Database) -> Result<SymbolIdMap> {
    let mut map = SymbolIdMap::new();
    let mut stmt = db.conn().prepare(
        "SELECT f.path, s.qualified_name, s.id
         FROM symbols s
         JOIN files f ON f.id = s.file_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (path, qname, id) = row?;
        map.insert((path, qname), id);
    }
    Ok(map)
}

#[cfg(test)]
#[path = "write_tests.rs"]
mod tests;
