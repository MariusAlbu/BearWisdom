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
use std::collections::HashMap;
use tracing::{debug, warn};

/// Maps relative_path → SQLite file row ID.
pub type FileIdMap = HashMap<String, i64>;

/// Maps (relative_path, qualified_name) → SQLite symbol row ID.
pub type SymbolIdMap = HashMap<(String, String), i64>;

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
                "INSERT INTO files (path, hash, language, last_indexed, mtime, size)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(path) DO UPDATE SET
                   hash = excluded.hash,
                   language = excluded.language,
                   last_indexed = excluded.last_indexed,
                   mtime = excluded.mtime,
                   size = excluded.size
                 RETURNING id",
            )
            .context("Failed to prepare file upsert")?
            .query_row(
                rusqlite::params![pf.path, pf.content_hash, pf.language, now, pf.mtime, pf.size as i64],
                |r| r.get(0),
            )
            .with_context(|| format!("Failed to upsert file {}", pf.path))?;

        file_id_map.insert(pf.path.clone(), file_id);

        // Delete existing symbols (ON CONFLICT upsert doesn't cascade-delete).
        tx.prepare_cached("DELETE FROM symbols WHERE file_id = ?1")
            .context("Failed to prepare symbol delete")?
            .execute([file_id])?;

        // Delete existing imports (not cascaded by symbols delete).
        tx.prepare_cached("DELETE FROM imports WHERE file_id = ?1")
            .context("Failed to prepare import delete")?
            .execute([file_id])?;

        // Insert all symbols.
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
            .with_context(|| {
                format!("Failed to insert symbol {} in {}", sym.qualified_name, pf.path)
            })?;

            let sym_id = tx.last_insert_rowid();
            symbol_id_map.insert((pf.path.clone(), sym.qualified_name.clone()), sym_id);
        }

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

        // Insert import records.
        for r in &pf.refs {
            if r.kind != crate::types::EdgeKind::Imports {
                continue;
            }
            tx.prepare_cached(
                "INSERT INTO imports (file_id, imported_name, module_path, alias, line)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .context("Failed to prepare import insert")?
            .execute(rusqlite::params![
                file_id,
                r.target_name,
                r.module.as_deref(),
                Option::<&str>::None,
                r.line,
            ])
            .with_context(|| {
                format!("Failed to insert import '{}' in {}", r.target_name, pf.path)
            })?;
        }
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
pub fn delete_files(db: &Database, paths: &[String]) -> Result<Vec<(i64, String)>> {
    let conn = db.conn();
    let mut deleted = Vec::new();

    for rel_path in paths {
        let file_id: Option<i64> = conn
            .query_row(
                "SELECT id FROM files WHERE path = ?1",
                [rel_path],
                |r| r.get(0),
            )
            .ok();

        if let Some(file_id) = file_id {
            // Virtual table cleanup (not covered by CASCADE).
            let _ = crate::search::vector_store::delete_file_vectors(conn, file_id);

            // CASCADE handles symbols, edges, imports, routes, code_chunks,
            // connection_points, etc.
            conn.execute("DELETE FROM files WHERE id = ?1", [file_id])?;

            // Manual cleanup for tables without FK to files.
            let _ = conn.execute("DELETE FROM fts_content WHERE rowid = ?1", [file_id]);
            let _ = conn.execute(
                "DELETE FROM flow_edges WHERE source_file_id = ?1 OR target_file_id = ?1",
                [file_id],
            );

            deleted.push((file_id, rel_path.clone()));
            debug!("Deleted file from index: {rel_path}");
        }
    }

    Ok(deleted)
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
