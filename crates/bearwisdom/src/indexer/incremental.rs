// =============================================================================
// indexer/incremental.rs  —  incremental + targeted re-indexing
//
// All non-full index paths flow through this module.  Each uses a different
// change detection strategy (see changeset.rs) but feeds into the same
// shared write pipeline (write.rs):
//
//   • incremental_index — HashDiff (walk + SHA-256, for non-git repos)
//   • git_reindex       — GitDiff  (git diff, preferred for git repos)
//   • reindex_files     — FileEvents (IDE/watcher push, fastest path)
//
// After writing changed files, all paths run blast-radius analysis to find
// dependent files that need re-resolution, then re-resolve the combined set.
// =============================================================================

use crate::db::Database;
use crate::indexer::changeset::{self, ChangeSet};
use crate::indexer::full;
use crate::indexer::ref_cache::RefCache;
use crate::indexer::resolve;
use crate::indexer::write;
use crate::languages;
use crate::types::ParsedFile;
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::{debug, info, warn};

// Re-export watcher event types from changeset (public API).
pub use crate::indexer::changeset::{ChangeKind, FileChangeEvent};

// ---------------------------------------------------------------------------
// Stats
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

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Incrementally update the index using hash-based change detection.
///
/// Walks the project tree and compares SHA-256 hashes against the database.
/// This is the fallback for non-git repos.  For git repos, prefer
/// `git_reindex` which avoids reading every file.
pub fn incremental_index(
    db: &mut Database,
    project_root: &Path,
    ref_cache: Option<&Arc<Mutex<RefCache>>>,
) -> Result<IncrementalStats> {
    let start = Instant::now();
    info!("Starting incremental index (HashDiff) of {}", project_root.display());

    let cs = changeset::hash_diff(db, project_root)?;
    run_incremental_pipeline(db, project_root, cs, start, ref_cache)
}

/// Incrementally update the index using git diff change detection.
///
/// Uses `git diff --name-status` between the last indexed commit and HEAD.
/// Falls back to hash-based detection if not a git repo or the indexed
/// commit is unreachable (force push, rebase).
pub fn git_reindex(
    db: &mut Database,
    project_root: &Path,
    ref_cache: Option<&Arc<Mutex<RefCache>>>,
) -> Result<IncrementalStats> {
    let start = Instant::now();
    info!("Starting incremental index (GitDiff) of {}", project_root.display());

    let cs = changeset::git_diff(db, project_root)?;
    run_incremental_pipeline(db, project_root, cs, start, ref_cache)
}

/// Re-index specific files from IDE/watcher events.
///
/// This is the fast path — no tree walk, no hashing.  The caller supplies
/// exactly which files changed and how.
pub fn reindex_files(
    db: &mut Database,
    project_root: &Path,
    changes: &[FileChangeEvent],
    ref_cache: Option<&Arc<Mutex<RefCache>>>,
) -> Result<IncrementalStats> {
    let start = Instant::now();

    if changes.is_empty() {
        return Ok(IncrementalStats::default());
    }

    info!("Targeted reindex: {} file changes", changes.len());

    let cs = changeset::from_file_events(project_root, changes)?;
    run_incremental_pipeline(db, project_root, cs, start, ref_cache)
}

// ---------------------------------------------------------------------------
// Shared incremental pipeline
// ---------------------------------------------------------------------------

/// The unified incremental pipeline.  All non-full index paths call this.
///
/// Steps:
///   1. Compute blast radius BEFORE any mutations (edges will be deleted by CASCADE).
///   2. Delete removed files.
///   3. Parse changed files (parallel via Rayon).
///   4. Write files + symbols via shared pipeline.
///   5. Update FTS content + code chunks.
///   6. Load full symbol map (changed + unchanged).
///   7. Parse + re-resolve blast-radius affected files.
///   8. Store indexed_commit in metadata.
fn run_incremental_pipeline(
    db: &mut Database,
    project_root: &Path,
    cs: ChangeSet,
    start: Instant,
    ref_cache: Option<&Arc<Mutex<RefCache>>>,
) -> Result<IncrementalStats> {
    let mut stats = IncrementalStats {
        files_added: cs.added.len() as u32,
        files_modified: cs.modified.len() as u32,
        files_deleted: cs.deleted.len() as u32,
        files_unchanged: cs.unchanged,
        ..Default::default()
    };

    if cs.is_empty() {
        info!("No changes detected, index is up to date");
        stats.duration_ms = start.elapsed().as_millis() as u64;
        return Ok(stats);
    }

    // Collect paths for blast radius lookup BEFORE any DB mutations.
    let changed_paths: HashSet<String> = cs
        .added
        .iter()
        .chain(cs.modified.iter())
        .map(|w| w.relative_path.clone())
        .chain(cs.deleted.iter().cloned())
        .collect();

    // --- Step 1: Blast radius (find dependents before CASCADE deletes edges) ---
    let dependent_paths = find_dependent_files(db, &changed_paths)?;
    if !dependent_paths.is_empty() {
        debug!(
            "Blast radius: {} files depend on changed files",
            dependent_paths.len()
        );
    }

    // --- Step 2: Delete removed files ---
    if !cs.deleted.is_empty() {
        write::delete_files(db, &cs.deleted)?;
    }

    // --- Step 3: Invalidate ref cache ---
    if let Some(rc) = ref_cache {
        let mut guard = rc.lock().unwrap();
        for path in &cs.deleted {
            guard.invalidate(path);
        }
        for w in cs.added.iter().chain(cs.modified.iter()) {
            guard.invalidate(&w.relative_path);
        }
    }

    // --- Step 4: Parse changed files (parallel) ---
    let files_to_parse: Vec<_> = cs.added.into_iter().chain(cs.modified).collect();
    let registry = languages::default_registry();
    let parse_results: Vec<Result<ParsedFile>> = files_to_parse
        .par_iter()
        .map(|w| full::parse_file(w, registry))
        .collect();

    let mut parsed: Vec<ParsedFile> = Vec::with_capacity(files_to_parse.len());
    for (walked, result) in files_to_parse.iter().zip(parse_results) {
        match result {
            Ok(pf) => parsed.push(pf),
            Err(e) => warn!("Failed to parse {}: {e}", walked.relative_path),
        }
    }

    if parsed.is_empty() && cs.deleted.is_empty() {
        stats.duration_ms = start.elapsed().as_millis() as u64;
        return Ok(stats);
    }

    // --- Step 5: Write files + symbols (shared pipeline) ---
    let file_id_map = if !parsed.is_empty() {
        let (fmap, smap) =
            write::write_parsed_files(db, &parsed).context("Failed to write index")?;
        stats.symbols_written = smap.len() as u32;
        let _ = smap; // symbol IDs for parsed files are subset of full map below
        fmap
    } else {
        Default::default()
    };

    // --- Step 6: FTS + chunks (shared pipeline) ---
    if !parsed.is_empty() {
        write::update_fts_content(db, &parsed, &file_id_map)?;
        write::update_chunks(db, &parsed, &file_id_map, false)?;
    }

    // --- Step 7: Load full symbol map (post-commit) ---
    // Needed by the heuristic resolver to match targets from any file.
    // The engine resolver gets completeness from SymbolIndex::augment_from_db.
    let symbol_id_map = write::load_symbol_id_map(db)?;

    // --- Step 8: Blast radius re-resolution ---
    // Find files with unresolved refs matching symbols from changed files.
    let new_symbol_names: HashSet<String> = parsed
        .iter()
        .flat_map(|pf| pf.symbols.iter().map(|s| s.name.clone()))
        .collect();
    let newly_resolvable =
        find_newly_resolvable_files(db, &new_symbol_names, &changed_paths)?;

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

        // Parse affected files (source unchanged but need refs for resolver).
        let affected_walked: Vec<_> = all_affected
            .iter()
            .filter_map(|rel_path| {
                let abs_path = project_root.join(rel_path);
                if !abs_path.exists() {
                    return None;
                }
                let language = crate::walker::detect_language(&abs_path)?;
                Some(crate::walker::WalkedFile {
                    relative_path: rel_path.clone(),
                    absolute_path: abs_path,
                    language,
                })
            })
            .collect();

        let affected_results: Vec<Result<ParsedFile>> = affected_walked
            .par_iter()
            .map(|w| full::parse_file(w, registry))
            .collect();

        let mut affected_parsed: Vec<ParsedFile> = Vec::new();
        for (walked, result) in affected_walked.iter().zip(affected_results) {
            match result {
                Ok(pf) => affected_parsed.push(pf),
                Err(e) => warn!("Failed to parse dependent {}: {e}", walked.relative_path),
            }
        }

        // Clean stale unresolved/external refs for affected files — batched via temp table.
        if !affected_parsed.is_empty() {
            db.conn().execute(
                "CREATE TEMP TABLE IF NOT EXISTS _affected_paths (path TEXT PRIMARY KEY)",
                [],
            )?;
            db.conn().execute("DELETE FROM _affected_paths", [])?;

            let mut ins = db
                .prepare("INSERT OR IGNORE INTO _affected_paths (path) VALUES (?1)")?;
            for pf in &affected_parsed {
                ins.execute([&pf.path])?;
            }
            drop(ins);

            db.conn().execute(
                "DELETE FROM unresolved_refs WHERE source_id IN (
                    SELECT s.id FROM symbols s
                    JOIN files f ON s.file_id = f.id
                    JOIN _affected_paths ap ON ap.path = f.path
                )",
                [],
            )?;

            db.conn().execute(
                "DELETE FROM external_refs WHERE source_id IN (
                    SELECT s.id FROM symbols s
                    JOIN files f ON s.file_id = f.id
                    JOIN _affected_paths ap ON ap.path = f.path
                )",
                [],
            )?;

            db.conn().execute("DELETE FROM _affected_paths", [])?;
        }

        // Extend parsed with affected files for combined resolution.
        parsed.extend(affected_parsed);
    }

    // --- Step 9: Cross-file resolution ---
    let project_ctx = super::project_context::build_project_context(project_root);
    let rstats = resolve::resolve_and_write_incremental(db, &parsed, &symbol_id_map, Some(&project_ctx))
        .context("Failed to resolve references")?;
    stats.edges_written = rstats.resolved as u32;
    info!("Resolved {} edges for changed files", rstats.resolved);

    // --- Step 10: Store indexed commit ---
    if let Some(commit) = cs.commit {
        if let Err(e) = changeset::set_meta(db, "indexed_commit", &commit) {
            warn!("Failed to store indexed_commit: {e}");
        }
    }

    stats.duration_ms = start.elapsed().as_millis() as u64;
    info!(
        "Incremental index complete in {:.2}s: +{} ~{} -{} re-resolved:{} symbols:{} edges:{}",
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
// Blast-radius helpers
// ---------------------------------------------------------------------------

/// Find files that depend on the given source files via the edges table.
///
/// Returns file paths (not in `source_paths`) whose symbols have outgoing
/// edges pointing TO symbols in the source files.  These dependents need
/// re-resolution when source files are modified or deleted, because CASCADE
/// will delete the edges when the target symbols are replaced.
///
/// Uses a temp table to batch all paths into a single 6-way JOIN, avoiding
/// an N-query loop.
fn find_dependent_files(
    db: &Database,
    source_paths: &HashSet<String>,
) -> Result<HashSet<String>> {
    if source_paths.is_empty() {
        return Ok(HashSet::new());
    }

    db.conn().execute(
        "CREATE TEMP TABLE IF NOT EXISTS _changed_paths (path TEXT PRIMARY KEY)",
        [],
    )?;
    db.conn().execute("DELETE FROM _changed_paths", [])?;

    {
        let mut ins = db
            .prepare("INSERT OR IGNORE INTO _changed_paths (path) VALUES (?1)")?;
        for path in source_paths {
            ins.execute([path.as_str()])?;
        }
    }

    let mut stmt = db.conn().prepare(
        "SELECT DISTINCT f_dep.path
         FROM edges e
         JOIN symbols s_target ON e.target_id = s_target.id
         JOIN files   f_target ON s_target.file_id = f_target.id
         JOIN symbols s_dep    ON e.source_id = s_dep.id
         JOIN files   f_dep    ON s_dep.file_id = f_dep.id
         JOIN _changed_paths cp ON cp.path = f_target.path
         WHERE f_dep.path NOT IN (SELECT path FROM _changed_paths)",
    )?;

    let mut dependents = HashSet::new();
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    for row in rows {
        dependents.insert(row?);
    }

    db.conn().execute("DELETE FROM _changed_paths", [])?;
    Ok(dependents)
}

/// Find files with unresolved references whose `target_name` matches any of
/// the given symbol names.  These files may now be resolvable because the
/// target symbols have been added or restored.
///
/// Uses a temp table to batch all names into a single JOIN, avoiding an
/// N-query loop.
fn find_newly_resolvable_files(
    db: &Database,
    symbol_names: &HashSet<String>,
    exclude_paths: &HashSet<String>,
) -> Result<HashSet<String>> {
    if symbol_names.is_empty() {
        return Ok(HashSet::new());
    }

    db.conn().execute(
        "CREATE TEMP TABLE IF NOT EXISTS _changed_names (name TEXT PRIMARY KEY)",
        [],
    )?;
    db.conn().execute("DELETE FROM _changed_names", [])?;

    {
        let mut ins = db
            .prepare("INSERT OR IGNORE INTO _changed_names (name) VALUES (?1)")?;
        for name in symbol_names {
            ins.execute([name.as_str()])?;
        }
    }

    // Exclude paths that were themselves changed — they are re-parsed directly.
    db.conn().execute(
        "CREATE TEMP TABLE IF NOT EXISTS _exclude_paths (path TEXT PRIMARY KEY)",
        [],
    )?;
    db.conn().execute("DELETE FROM _exclude_paths", [])?;

    {
        let mut ins = db
            .prepare("INSERT OR IGNORE INTO _exclude_paths (path) VALUES (?1)")?;
        for path in exclude_paths {
            ins.execute([path.as_str()])?;
        }
    }

    let mut stmt = db.conn().prepare(
        "SELECT DISTINCT f.path
         FROM unresolved_refs ur
         JOIN symbols s ON ur.source_id = s.id
         JOIN files   f ON s.file_id = f.id
         JOIN _changed_names cn ON cn.name = ur.target_name
         WHERE f.path NOT IN (SELECT path FROM _exclude_paths)",
    )?;

    let mut resolvable = HashSet::new();
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    for row in rows {
        resolvable.insert(row?);
    }

    db.conn().execute("DELETE FROM _changed_names", [])?;
    db.conn().execute("DELETE FROM _exclude_paths", [])?;
    Ok(resolvable)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "incremental_tests.rs"]
mod tests;
