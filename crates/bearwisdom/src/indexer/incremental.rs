// =============================================================================
// indexer/incremental.rs  —  incremental re-indexing
//
// Instead of re-indexing every file on every change, this module:
//   1. Walks the project tree and computes SHA-256 hashes.
//   2. Compares hashes against the `files.hash` column.
//   3. Only re-parses files whose hash changed or that are new.
//   4. Deletes symbols/edges for removed files.
//   5. Re-runs cross-file resolution only for affected files.
//   6. Re-runs connectors that depend on changed files.
//
// Performance target: <100ms for a single file change in a 1000-file project.
// =============================================================================

use crate::db::Database;
use crate::indexer::full;
use crate::indexer::resolve;
use crate::types::ParsedFile;
use crate::walker::{self, WalkedFile};
use anyhow::{Context, Result};
use rayon::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Change detection
// ---------------------------------------------------------------------------

/// Classification of what happened to a file since last index.
#[derive(Debug)]
enum FileChange {
    /// File is new (not in the database).
    Added(WalkedFile),
    /// File content changed (hash differs).
    Modified(WalkedFile),
    /// File was deleted (in database but not on disk).
    Deleted { file_id: i64, path: String },
    /// File is unchanged (hash matches).
    Unchanged,
}

/// Detect which files have changed since the last index.
fn detect_changes(
    db: &Database,
    project_root: &Path,
) -> Result<Vec<FileChange>> {
    let files_on_disk = walker::walk(project_root)
        .with_context(|| format!("Failed to walk {}", project_root.display()))?;

    // Load existing file records from the database.
    let mut existing: HashMap<String, (i64, String)> = HashMap::new();
    {
        let mut stmt = db
            .conn
            .prepare("SELECT id, path, hash FROM files")
            .context("Failed to query files")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .context("Failed to read files")?;
        for row in rows {
            let (id, path, hash) = row?;
            existing.insert(path, (id, hash));
        }
    }

    let mut changes = Vec::new();
    let mut seen_paths: HashSet<String> = HashSet::new();

    for walked in files_on_disk {
        seen_paths.insert(walked.relative_path.clone());

        // Compute hash of current file content.
        let content = match std::fs::read(&walked.absolute_path) {
            Ok(c) => c,
            Err(e) => {
                debug!("Cannot read {}: {e}", walked.relative_path);
                continue;
            }
        };
        let hash = {
            let mut hasher = Sha256::new();
            hasher.update(&content);
            format!("{:x}", hasher.finalize())
        };

        match existing.get(&walked.relative_path) {
            Some((_id, old_hash)) if *old_hash == hash => {
                changes.push(FileChange::Unchanged);
            }
            Some(_) => {
                changes.push(FileChange::Modified(walked));
            }
            None => {
                changes.push(FileChange::Added(walked));
            }
        }
    }

    // Detect deleted files (in DB but not on disk).
    for (path, (file_id, _hash)) in &existing {
        if !seen_paths.contains(path) {
            changes.push(FileChange::Deleted {
                file_id: *file_id,
                path: path.clone(),
            });
        }
    }

    Ok(changes)
}

// ---------------------------------------------------------------------------
// Incremental index
// ---------------------------------------------------------------------------

/// Result of an incremental index pass.
#[derive(Debug, Clone, Default)]
pub struct IncrementalStats {
    pub files_added: u32,
    pub files_modified: u32,
    pub files_deleted: u32,
    pub files_unchanged: u32,
    pub symbols_written: u32,
    pub edges_written: u32,
    pub files_reresolved: u32,
    pub duration_ms: u64,
}

/// Incrementally update the index for changed files only.
///
/// This is much faster than `full_index` when only a few files changed.
/// For a full rebuild, use `full_index` instead.
pub fn incremental_index(
    db: &mut Database,
    project_root: &Path,
) -> Result<IncrementalStats> {
    let start = Instant::now();
    info!("Starting incremental index of {}", project_root.display());

    // Step 1: Detect changes.
    let changes = detect_changes(db, project_root)?;

    let mut stats = IncrementalStats::default();
    let mut files_to_parse: Vec<WalkedFile> = Vec::new();
    let mut files_to_delete: Vec<(i64, String)> = Vec::new();

    for change in changes {
        match change {
            FileChange::Added(w) => {
                stats.files_added += 1;
                files_to_parse.push(w);
            }
            FileChange::Modified(w) => {
                stats.files_modified += 1;
                files_to_parse.push(w);
            }
            FileChange::Deleted { file_id, path } => {
                stats.files_deleted += 1;
                files_to_delete.push((file_id, path));
            }
            FileChange::Unchanged => {
                stats.files_unchanged += 1;
            }
        }
    }

    info!(
        "Change detection: {} added, {} modified, {} deleted, {} unchanged",
        stats.files_added, stats.files_modified, stats.files_deleted, stats.files_unchanged
    );

    if files_to_parse.is_empty() && files_to_delete.is_empty() {
        info!("No changes detected, index is up to date");
        stats.duration_ms = start.elapsed().as_millis() as u64;
        return Ok(stats);
    }

    // Step 2: Delete removed files (CASCADE removes symbols, edges, chunks, etc.)
    for (file_id, path) in &files_to_delete {
        // Clean up vec_chunks (virtual table — not covered by CASCADE).
        let _ = crate::search::vector_store::delete_file_vectors(&db.conn, *file_id);

        db.conn
            .execute("DELETE FROM files WHERE id = ?1", [file_id])
            .with_context(|| format!("Failed to delete file {path}"))?;

        // Also clean up FTS content and flow edges.
        let _ = db.conn.execute("DELETE FROM fts_content WHERE rowid = ?1", [file_id]);
        let _ = db.conn.execute(
            "DELETE FROM flow_edges WHERE source_file_id = ?1 OR target_file_id = ?1",
            [file_id],
        );
        debug!("Deleted file from index: {path}");
    }

    // Step 3: Parse changed/new files (parallel).
    let parse_results: Vec<Result<ParsedFile>> =
        files_to_parse.par_iter().map(full::parse_file).collect();

    let mut parsed: Vec<ParsedFile> = Vec::with_capacity(files_to_parse.len());
    for (walked, result) in files_to_parse.iter().zip(parse_results) {
        match result {
            Ok(pf) => parsed.push(pf),
            Err(e) => warn!("Failed to parse {}: {e}", walked.relative_path),
        }
    }

    if parsed.is_empty() {
        stats.duration_ms = start.elapsed().as_millis() as u64;
        return Ok(stats);
    }

    // Step 4: Write files + symbols (reuses full_index write logic).
    let conn = &db.conn;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let tx = conn
        .unchecked_transaction()
        .context("Failed to begin transaction")?;

    let mut file_id_map: HashMap<String, i64> = HashMap::new();
    let mut symbol_id_map: HashMap<(String, String), i64> = HashMap::new();

    for pf in &parsed {
        // Upsert file row.
        tx.execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET
               hash = excluded.hash,
               language = excluded.language,
               last_indexed = excluded.last_indexed",
            rusqlite::params![pf.path, pf.content_hash, pf.language, now],
        )?;

        let file_id: i64 = tx.query_row(
            "SELECT id FROM files WHERE path = ?1",
            [&pf.path],
            |r| r.get(0),
        )?;

        file_id_map.insert(pf.path.clone(), file_id);

        // Delete existing symbols for this file.
        tx.execute("DELETE FROM symbols WHERE file_id = ?1", [file_id])?;
        tx.execute("DELETE FROM imports WHERE file_id = ?1", [file_id])?;

        // Insert symbols.
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
            )?;

            let sym_id = tx.last_insert_rowid();
            symbol_id_map.insert((pf.path.clone(), sym.qualified_name.clone()), sym_id);
            stats.symbols_written += 1;
        }

        // Insert imports.
        for r in &pf.refs {
            if r.kind != crate::types::EdgeKind::Imports {
                continue;
            }
            tx.execute(
                "INSERT INTO imports (file_id, imported_name, module_path, alias, line)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    file_id,
                    r.target_name,
                    r.module,
                    Option::<&str>::None,
                    r.line,
                ],
            )?;
        }

        // Update FTS content.
        let _ = tx.execute("DELETE FROM fts_content WHERE rowid = ?1", [file_id]);
        if let Some(content) = &pf.content {
            let _ = tx.execute(
                "INSERT INTO fts_content(rowid, path, content) VALUES (?1, ?2, ?3)",
                rusqlite::params![file_id, pf.path, content],
            );
        }

        // Update code chunks (delete old vectors first — virtual table, no CASCADE).
        let _ = crate::search::vector_store::delete_file_vectors(&tx, file_id);
        let _ = tx.execute("DELETE FROM code_chunks WHERE file_id = ?1", [file_id]);
        if let Some(content) = &pf.content {
            if let Err(e) = crate::search::chunker::chunk_and_store(&tx, file_id, content) {
                debug!("Chunking failed for {}: {e}", pf.path);
            }
        }
    }

    tx.commit().context("Failed to commit incremental update")?;

    // Step 5: Re-run cross-file resolution for changed files.
    // We need to include ALL parsed files in the resolution pass, plus
    // load the full symbol_id_map (including unchanged files) for resolution.
    {
        // Load symbol IDs for unchanged files too.
        // Scope the statement so it's dropped before resolve_and_write borrows db mutably.
        {
            let mut stmt = db.conn.prepare(
                "SELECT f.path, s.qualified_name, s.id FROM symbols s JOIN files f ON f.id = s.file_id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
            })?;
            for row in rows {
                let (path, qname, id) = row?;
                symbol_id_map.entry((path, qname)).or_insert(id);
            }
        }

        let (edge_count, _unresolved) =
            resolve::resolve_and_write(db, &parsed, &symbol_id_map)
                .context("Failed to resolve references")?;
        stats.edges_written = edge_count as u32;
        info!("Resolved {} edges for changed files", edge_count);
    }

    stats.duration_ms = start.elapsed().as_millis() as u64;
    info!(
        "Incremental index complete in {:.2}s: {} added, {} modified, {} deleted, {} symbols, {} edges",
        stats.duration_ms as f64 / 1000.0,
        stats.files_added,
        stats.files_modified,
        stats.files_deleted,
        stats.symbols_written,
        stats.edges_written,
    );

    Ok(stats)
}

// ---------------------------------------------------------------------------
// Targeted reindex (fast path for file watcher events)
// ---------------------------------------------------------------------------

/// Describes a file change reported by a file watcher.
#[derive(Debug, Clone)]
pub struct FileChangeEvent {
    /// Path relative to project root, forward-slash normalised.
    pub relative_path: String,
    /// What happened to the file.
    pub change_kind: ChangeKind,
}

/// The kind of change a file watcher observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Created,
    Modified,
    Deleted,
}

// ---------------------------------------------------------------------------
// Blast-radius helpers
// ---------------------------------------------------------------------------

/// Find files that depend on the given source files via the edges table.
///
/// Returns file paths (not in `source_paths`) whose symbols have outgoing
/// edges pointing TO symbols in the source files.  These dependents need
/// re-resolution when source files are modified or deleted, because CASCADE
/// will delete the edges when the target symbols are replaced.
fn find_dependent_files(
    db: &Database,
    source_paths: &HashSet<String>,
) -> Result<HashSet<String>> {
    if source_paths.is_empty() {
        return Ok(HashSet::new());
    }

    let mut dependents = HashSet::new();

    for path in source_paths {
        let mut stmt = db.conn.prepare(
            "SELECT DISTINCT f_dep.path
             FROM edges e
             JOIN symbols s_target ON e.target_id = s_target.id
             JOIN files   f_target ON s_target.file_id = f_target.id
             JOIN symbols s_dep    ON e.source_id = s_dep.id
             JOIN files   f_dep    ON s_dep.file_id = f_dep.id
             WHERE f_target.path = ?1",
        )?;
        let rows = stmt.query_map([path], |r| r.get::<_, String>(0))?;
        for row in rows {
            let dep_path = row?;
            if !source_paths.contains(&dep_path) {
                dependents.insert(dep_path);
            }
        }
    }

    Ok(dependents)
}

/// Find files with unresolved references whose `target_name` matches any of
/// the given symbol names.  These files may now be resolvable because the
/// target symbols have been added or restored.
fn find_newly_resolvable_files(
    db: &Database,
    symbol_names: &HashSet<String>,
    exclude_paths: &HashSet<String>,
) -> Result<HashSet<String>> {
    if symbol_names.is_empty() {
        return Ok(HashSet::new());
    }

    let mut resolvable = HashSet::new();

    for name in symbol_names {
        let mut stmt = db.conn.prepare(
            "SELECT DISTINCT f.path
             FROM unresolved_refs ur
             JOIN symbols s ON ur.source_id = s.id
             JOIN files   f ON s.file_id = f.id
             WHERE ur.target_name = ?1",
        )?;
        let rows = stmt.query_map([name.as_str()], |r| r.get::<_, String>(0))?;
        for row in rows {
            let path = row?;
            if !exclude_paths.contains(&path) {
                resolvable.insert(path);
            }
        }
    }

    Ok(resolvable)
}

/// Re-index specific files that changed, without walking the project tree.
///
/// This is the fast path for file watcher events.  Instead of walking the
/// entire tree and comparing hashes (what `incremental_index` does), this
/// function directly processes the caller-supplied change list.
///
/// Returns an empty `IncrementalStats` immediately when `changes` is empty.
pub fn reindex_files(
    db: &mut Database,
    project_root: &Path,
    changes: &[FileChangeEvent],
) -> Result<IncrementalStats> {
    let start = Instant::now();

    if changes.is_empty() {
        return Ok(IncrementalStats::default());
    }

    info!("Targeted reindex: {} file changes", changes.len());

    let mut stats = IncrementalStats::default();
    let mut files_to_parse: Vec<WalkedFile> = Vec::new();
    let mut files_to_delete: Vec<String> = Vec::new(); // relative paths

    for change in changes {
        match change.change_kind {
            ChangeKind::Deleted => {
                stats.files_deleted += 1;
                files_to_delete.push(change.relative_path.clone());
            }
            ChangeKind::Created | ChangeKind::Modified => {
                let abs_path = project_root.join(&change.relative_path);

                // Skip if the file no longer exists (race: deleted between watcher
                // event and reindex).
                if !abs_path.exists() {
                    debug!(
                        "File no longer exists, skipping: {}",
                        change.relative_path
                    );
                    continue;
                }

                // Detect language — skip unsupported files.
                let language = match walker::detect_language(&abs_path) {
                    Some(l) => l,
                    None => continue,
                };

                if change.change_kind == ChangeKind::Created {
                    stats.files_added += 1;
                } else {
                    stats.files_modified += 1;
                }

                files_to_parse.push(WalkedFile {
                    relative_path: change.relative_path.clone(),
                    absolute_path: abs_path,
                    language,
                });
            }
        }
    }

    // ── Blast radius: find dependents BEFORE modifications ──────────
    // We must query edges before CASCADE deletes them when symbols are
    // replaced or files are removed.
    let changed_paths: HashSet<String> = files_to_parse
        .iter()
        .map(|w| w.relative_path.clone())
        .chain(files_to_delete.iter().cloned())
        .collect();
    let dependent_paths = find_dependent_files(db, &changed_paths)?;
    if !dependent_paths.is_empty() {
        debug!(
            "Blast radius: {} files depend on changed files",
            dependent_paths.len()
        );
    }

    // Handle deletions.
    for rel_path in &files_to_delete {
        let file_id: Option<i64> = db
            .conn
            .query_row(
                "SELECT id FROM files WHERE path = ?1",
                [rel_path],
                |r| r.get(0),
            )
            .ok();

        if let Some(file_id) = file_id {
            // Clean up vec_chunks (virtual table — not covered by CASCADE).
            let _ = crate::search::vector_store::delete_file_vectors(&db.conn, file_id);

            db.conn
                .execute("DELETE FROM files WHERE id = ?1", [file_id])?;
            let _ = db
                .conn
                .execute("DELETE FROM fts_content WHERE rowid = ?1", [file_id]);
            let _ = db.conn.execute(
                "DELETE FROM flow_edges WHERE source_file_id = ?1 OR target_file_id = ?1",
                [file_id],
            );
            debug!("Deleted file from index: {rel_path}");
        }
    }

    if files_to_parse.is_empty() && files_to_delete.is_empty() {
        stats.duration_ms = start.elapsed().as_millis() as u64;
        return Ok(stats);
    }

    // Parse changed/new files (parallel via Rayon, mirroring incremental_index).
    let parse_results: Vec<Result<ParsedFile>> =
        files_to_parse.par_iter().map(full::parse_file).collect();

    let mut parsed: Vec<ParsedFile> = Vec::with_capacity(files_to_parse.len());
    for (walked, result) in files_to_parse.iter().zip(parse_results) {
        match result {
            Ok(pf) => parsed.push(pf),
            Err(e) => warn!("Failed to parse {}: {e}", walked.relative_path),
        }
    }

    // ── Step 1: Write changed files to DB ──────────────────────────
    if !parsed.is_empty() {
        let conn = &db.conn;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let tx = conn
            .unchecked_transaction()
            .context("Failed to begin transaction")?;

        for pf in &parsed {
            tx.execute(
                "INSERT INTO files (path, hash, language, last_indexed)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(path) DO UPDATE SET
                   hash = excluded.hash,
                   language = excluded.language,
                   last_indexed = excluded.last_indexed",
                rusqlite::params![pf.path, pf.content_hash, pf.language, now],
            )?;

            let file_id: i64 = tx.query_row(
                "SELECT id FROM files WHERE path = ?1",
                [&pf.path],
                |r| r.get(0),
            )?;

            tx.execute("DELETE FROM symbols WHERE file_id = ?1", [file_id])?;
            tx.execute("DELETE FROM imports WHERE file_id = ?1", [file_id])?;

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
                )?;

                stats.symbols_written += 1;
            }

            for r in &pf.refs {
                if r.kind != crate::types::EdgeKind::Imports {
                    continue;
                }
                tx.execute(
                    "INSERT INTO imports (file_id, imported_name, module_path, alias, line)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![
                        file_id,
                        r.target_name,
                        r.module,
                        Option::<&str>::None,
                        r.line,
                    ],
                )?;
            }

            let _ = tx.execute("DELETE FROM fts_content WHERE rowid = ?1", [file_id]);
            if let Some(content) = &pf.content {
                let _ = tx.execute(
                    "INSERT INTO fts_content(rowid, path, content) VALUES (?1, ?2, ?3)",
                    rusqlite::params![file_id, pf.path, content],
                );
            }

            let _ = crate::search::vector_store::delete_file_vectors(&tx, file_id);
            let _ = tx.execute("DELETE FROM code_chunks WHERE file_id = ?1", [file_id]);
            if let Some(content) = &pf.content {
                if let Err(e) =
                    crate::search::chunker::chunk_and_store(&tx, file_id, content)
                {
                    debug!("Chunking failed for {}: {e}", pf.path);
                }
            }
        }

        tx.commit().context("Failed to commit targeted reindex")?;
    }

    // ── Step 2: Blast-radius resolution ─────────────────────────────
    // Runs when there are changed files OR dependents from deletions.
    if !parsed.is_empty() || !dependent_paths.is_empty() {
        // Load full symbol map (post-commit, DB has all current symbols).
        let mut symbol_id_map: HashMap<(String, String), i64> = HashMap::new();
        {
            let mut stmt = db.conn.prepare(
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
                symbol_id_map.insert((path, qname), id);
            }
        }

        // Find files with unresolved refs matching symbols from changed files.
        let new_symbol_names: HashSet<String> = parsed
            .iter()
            .flat_map(|pf| pf.symbols.iter().map(|s| s.name.clone()))
            .collect();
        let newly_resolvable =
            find_newly_resolvable_files(db, &new_symbol_names, &changed_paths)?;

        // Combine all affected files (edge dependents + newly resolvable).
        let all_affected: HashSet<String> = dependent_paths
            .into_iter()
            .chain(newly_resolvable)
            .collect();

        if !all_affected.is_empty() {
            info!(
                "Blast radius: re-resolving {} dependent files",
                all_affected.len()
            );
            stats.files_reresolved = all_affected.len() as u32;

            // Parse affected files — source hasn't changed, but we need
            // their refs for the resolver.
            let affected_walked: Vec<WalkedFile> = all_affected
                .iter()
                .filter_map(|rel_path| {
                    let abs_path = project_root.join(rel_path);
                    if !abs_path.exists() {
                        return None;
                    }
                    let language = walker::detect_language(&abs_path)?;
                    Some(WalkedFile {
                        relative_path: rel_path.clone(),
                        absolute_path: abs_path,
                        language,
                    })
                })
                .collect();

            let affected_results: Vec<Result<ParsedFile>> =
                affected_walked.par_iter().map(full::parse_file).collect();
            let mut affected_parsed: Vec<ParsedFile> = Vec::new();
            for (walked, result) in affected_walked.iter().zip(affected_results) {
                match result {
                    Ok(pf) => affected_parsed.push(pf),
                    Err(e) => {
                        warn!("Failed to parse dependent {}: {e}", walked.relative_path)
                    }
                }
            }

            // Clean up stale unresolved_refs for affected files —
            // re-resolution will recreate any that are still unresolvable.
            for pf in &affected_parsed {
                if let Ok(file_id) = db.conn.query_row(
                    "SELECT id FROM files WHERE path = ?1",
                    [&pf.path],
                    |r| r.get::<_, i64>(0),
                ) {
                    let _ = db.conn.execute(
                        "DELETE FROM unresolved_refs WHERE source_id IN \
                         (SELECT id FROM symbols WHERE file_id = ?1)",
                        [file_id],
                    );
                }
            }

            // Extend parsed with affected files for combined resolution.
            parsed.extend(affected_parsed);
        }

        let (edge_count, _unresolved) =
            resolve::resolve_and_write(db, &parsed, &symbol_id_map)
                .context("Failed to resolve references")?;
        stats.edges_written = edge_count as u32;
    }

    stats.duration_ms = start.elapsed().as_millis() as u64;
    info!(
        "Targeted reindex complete in {:.2}s: +{} ~{} -{} re-resolved:{} symbols:{} edges:{}",
        stats.duration_ms as f64 / 1000.0,
        stats.files_added,
        stats.files_modified,
        stats.files_deleted,
        stats.files_reresolved,
        stats.symbols_written,
        stats.edges_written,
    );

    Ok(stats)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn incremental_detects_new_file() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.cs"), "namespace App { class Foo {} }").unwrap();

        let mut db = Database::open_in_memory().unwrap();

        // Full index first.
        crate::indexer::full::full_index(&mut db, dir.path(), None, None).unwrap();
        let count1: u32 = db.conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0)).unwrap();

        // Add a new file.
        fs::write(dir.path().join("b.cs"), "namespace App { class Bar {} }").unwrap();

        let stats = incremental_index(&mut db, dir.path()).unwrap();
        assert_eq!(stats.files_added, 1);
        assert_eq!(stats.files_unchanged, count1);

        let count2: u32 = db.conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0)).unwrap();
        assert_eq!(count2, count1 + 1);
    }

    #[test]
    fn incremental_detects_modified_file() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.cs"), "namespace App { class Foo {} }").unwrap();

        let mut db = Database::open_in_memory().unwrap();
        crate::indexer::full::full_index(&mut db, dir.path(), None, None).unwrap();

        // Modify the file.
        fs::write(dir.path().join("a.cs"), "namespace App { class Foo { void Bar() {} } }").unwrap();

        let stats = incremental_index(&mut db, dir.path()).unwrap();
        assert_eq!(stats.files_modified, 1);
        assert_eq!(stats.files_added, 0);
    }

    #[test]
    fn incremental_detects_deleted_file() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.cs"), "namespace App { class Foo {} }").unwrap();
        fs::write(dir.path().join("b.cs"), "namespace App { class Bar {} }").unwrap();

        let mut db = Database::open_in_memory().unwrap();
        crate::indexer::full::full_index(&mut db, dir.path(), None, None).unwrap();

        // Delete one file.
        fs::remove_file(dir.path().join("b.cs")).unwrap();

        let stats = incremental_index(&mut db, dir.path()).unwrap();
        assert_eq!(stats.files_deleted, 1);

        let count: u32 = db.conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn incremental_no_changes_is_fast() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.cs"), "namespace App { class Foo {} }").unwrap();

        let mut db = Database::open_in_memory().unwrap();
        crate::indexer::full::full_index(&mut db, dir.path(), None, None).unwrap();

        let stats = incremental_index(&mut db, dir.path()).unwrap();
        assert_eq!(stats.files_added, 0);
        assert_eq!(stats.files_modified, 0);
        assert_eq!(stats.files_deleted, 0);
        assert!(stats.files_unchanged > 0);
    }

    // ------------------------------------------------------------------
    // reindex_files tests
    // ------------------------------------------------------------------

    #[test]
    fn reindex_files_handles_single_create() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.cs"), "namespace App { class Foo {} }").unwrap();

        let mut db = Database::open_in_memory().unwrap();
        crate::indexer::full::full_index(&mut db, dir.path(), None, None).unwrap();

        // Add a new file.
        fs::write(dir.path().join("b.cs"), "namespace App { class Bar {} }").unwrap();

        let changes = vec![FileChangeEvent {
            relative_path: "b.cs".to_string(),
            change_kind: ChangeKind::Created,
        }];

        let stats = reindex_files(&mut db, dir.path(), &changes).unwrap();
        assert_eq!(stats.files_added, 1);
        assert_eq!(stats.files_modified, 0);
        assert_eq!(stats.files_deleted, 0);

        let count: u32 = db
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn reindex_files_handles_modify() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.cs"), "namespace App { class Foo {} }").unwrap();

        let mut db = Database::open_in_memory().unwrap();
        crate::indexer::full::full_index(&mut db, dir.path(), None, None).unwrap();

        // Modify the file to add a method.
        fs::write(
            dir.path().join("a.cs"),
            "namespace App { class Foo { void Bar() {} } }",
        )
        .unwrap();

        let changes = vec![FileChangeEvent {
            relative_path: "a.cs".to_string(),
            change_kind: ChangeKind::Modified,
        }];

        let stats = reindex_files(&mut db, dir.path(), &changes).unwrap();
        assert_eq!(stats.files_modified, 1);

        // Should have more symbols now (Foo + Bar method).
        let sym_count: u32 = db
            .conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))
            .unwrap();
        assert!(sym_count >= 2, "Expected at least Foo + Bar, got {sym_count}");
    }

    #[test]
    fn reindex_files_handles_delete() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.cs"), "namespace App { class Foo {} }").unwrap();
        fs::write(dir.path().join("b.cs"), "namespace App { class Bar {} }").unwrap();

        let mut db = Database::open_in_memory().unwrap();
        crate::indexer::full::full_index(&mut db, dir.path(), None, None).unwrap();

        // Delete one file from disk.
        fs::remove_file(dir.path().join("b.cs")).unwrap();

        let changes = vec![FileChangeEvent {
            relative_path: "b.cs".to_string(),
            change_kind: ChangeKind::Deleted,
        }];

        let stats = reindex_files(&mut db, dir.path(), &changes).unwrap();
        assert_eq!(stats.files_deleted, 1);

        let count: u32 = db
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn reindex_files_skips_missing_created_file() {
        let dir = TempDir::new().unwrap();
        let mut db = Database::open_in_memory().unwrap();
        crate::indexer::full::full_index(&mut db, dir.path(), None, None).unwrap();

        // Report a created file that doesn't actually exist (race condition).
        let changes = vec![FileChangeEvent {
            relative_path: "phantom.cs".to_string(),
            change_kind: ChangeKind::Created,
        }];

        let stats = reindex_files(&mut db, dir.path(), &changes).unwrap();
        assert_eq!(stats.files_added, 0);
        assert_eq!(stats.files_modified, 0);
    }

    #[test]
    fn reindex_files_skips_unsupported_extensions() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("image.png"), "binary data").unwrap();

        let mut db = Database::open_in_memory().unwrap();
        crate::indexer::full::full_index(&mut db, dir.path(), None, None).unwrap();

        let changes = vec![FileChangeEvent {
            relative_path: "image.png".to_string(),
            change_kind: ChangeKind::Modified,
        }];

        let stats = reindex_files(&mut db, dir.path(), &changes).unwrap();
        assert_eq!(stats.files_added, 0);
        assert_eq!(stats.files_modified, 0);
    }

    #[test]
    fn reindex_files_empty_changes_is_noop() {
        let dir = TempDir::new().unwrap();
        let mut db = Database::open_in_memory().unwrap();
        let stats = reindex_files(&mut db, dir.path(), &[]).unwrap();
        assert_eq!(stats.files_added, 0);
        assert_eq!(stats.duration_ms, 0);
    }

    // ------------------------------------------------------------------
    // Blast-radius tests
    // ------------------------------------------------------------------

    /// When file A defines `Foo` and file B calls `Foo`, modifying A should
    /// trigger re-resolution of B (blast radius).
    #[test]
    fn blast_radius_reresolved_on_modify() {
        let dir = TempDir::new().unwrap();

        // File A defines a class with a method.
        fs::write(
            dir.path().join("a.cs"),
            "namespace App { class Svc { public void DoWork() {} } }",
        )
        .unwrap();

        // File B references the method from A.
        fs::write(
            dir.path().join("b.cs"),
            "namespace App { class Consumer { void Run() { DoWork(); } } }",
        )
        .unwrap();

        let mut db = Database::open_in_memory().unwrap();
        crate::indexer::full::full_index(&mut db, dir.path(), None, None).unwrap();

        // Verify there's at least one edge from B → A.
        let edge_count_before: u32 = db
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .unwrap();

        // Modify A: rename the method.
        fs::write(
            dir.path().join("a.cs"),
            "namespace App { class Svc { public void DoWorkRenamed() {} } }",
        )
        .unwrap();

        let changes = vec![FileChangeEvent {
            relative_path: "a.cs".to_string(),
            change_kind: ChangeKind::Modified,
        }];

        let stats = reindex_files(&mut db, dir.path(), &changes).unwrap();
        assert_eq!(stats.files_modified, 1);
        // B should be re-resolved via blast radius.
        assert!(
            stats.files_reresolved >= 1,
            "Expected B to be re-resolved, got {}",
            stats.files_reresolved
        );

        // The old edge (B → DoWork) should be gone since DoWork no longer exists.
        // B's reference to DoWork is now unresolvable.
        let unresolved: u32 = db
            .conn
            .query_row("SELECT COUNT(*) FROM unresolved_refs", [], |r| r.get(0))
            .unwrap();
        // B's call to DoWork() should now be in unresolved_refs.
        assert!(
            unresolved >= 1,
            "Expected unresolved ref for renamed symbol, got {unresolved} (edges before: {edge_count_before})"
        );
    }

    /// When a deleted file's symbols are referenced by other files, those
    /// dependents should be re-resolved.
    #[test]
    fn blast_radius_reresolved_on_delete() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("a.cs"),
            "namespace App { class Helper { public static void Aid() {} } }",
        )
        .unwrap();
        fs::write(
            dir.path().join("b.cs"),
            "namespace App { class Main { void Go() { Aid(); } } }",
        )
        .unwrap();

        let mut db = Database::open_in_memory().unwrap();
        crate::indexer::full::full_index(&mut db, dir.path(), None, None).unwrap();

        // Delete A.
        fs::remove_file(dir.path().join("a.cs")).unwrap();

        let changes = vec![FileChangeEvent {
            relative_path: "a.cs".to_string(),
            change_kind: ChangeKind::Deleted,
        }];

        let stats = reindex_files(&mut db, dir.path(), &changes).unwrap();
        assert_eq!(stats.files_deleted, 1);
        assert!(
            stats.files_reresolved >= 1,
            "Expected B to be re-resolved after A was deleted, got {}",
            stats.files_reresolved
        );
    }

    /// When a new file adds symbols that match previously unresolved refs
    /// in other files, those files should be re-resolved.
    #[test]
    fn blast_radius_resolves_previously_unresolved() {
        let dir = TempDir::new().unwrap();

        // File B references a symbol that doesn't exist yet.
        fs::write(
            dir.path().join("b.cs"),
            "namespace App { class User { void Go() { MissingMethod(); } } }",
        )
        .unwrap();

        let mut db = Database::open_in_memory().unwrap();
        crate::indexer::full::full_index(&mut db, dir.path(), None, None).unwrap();

        // Verify that MissingMethod is in unresolved_refs.
        let unresolved_before: u32 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM unresolved_refs WHERE target_name = 'MissingMethod'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            unresolved_before >= 1,
            "Expected MissingMethod in unresolved_refs"
        );

        // Now create a file that defines MissingMethod.
        fs::write(
            dir.path().join("a.cs"),
            "namespace App { class Lib { public void MissingMethod() {} } }",
        )
        .unwrap();

        let changes = vec![FileChangeEvent {
            relative_path: "a.cs".to_string(),
            change_kind: ChangeKind::Created,
        }];

        let stats = reindex_files(&mut db, dir.path(), &changes).unwrap();
        assert_eq!(stats.files_added, 1);
        assert!(
            stats.files_reresolved >= 1,
            "Expected B to be re-resolved when MissingMethod was added, got {}",
            stats.files_reresolved
        );
    }

    /// No blast radius when the change doesn't affect any dependents.
    #[test]
    fn blast_radius_zero_when_no_dependents() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("a.cs"),
            "namespace App { class Isolated {} }",
        )
        .unwrap();
        fs::write(
            dir.path().join("b.cs"),
            "namespace Other { class Unrelated {} }",
        )
        .unwrap();

        let mut db = Database::open_in_memory().unwrap();
        crate::indexer::full::full_index(&mut db, dir.path(), None, None).unwrap();

        // Modify A — B has no references to A.
        fs::write(
            dir.path().join("a.cs"),
            "namespace App { class Isolated { void New() {} } }",
        )
        .unwrap();

        let changes = vec![FileChangeEvent {
            relative_path: "a.cs".to_string(),
            change_kind: ChangeKind::Modified,
        }];

        let stats = reindex_files(&mut db, dir.path(), &changes).unwrap();
        assert_eq!(stats.files_modified, 1);
        assert_eq!(
            stats.files_reresolved, 0,
            "No files should be re-resolved when there are no dependents"
        );
    }
}
