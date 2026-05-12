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

    crate::languages::c_lang::macro_catalog::begin_index_session(project_root);
    let _macro_session_guard = MacroSessionGuard;

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

    crate::languages::c_lang::macro_catalog::begin_index_session(project_root);
    let _macro_session_guard = MacroSessionGuard;

    let cs = changeset::git_diff(db, project_root)?;
    run_incremental_pipeline(db, project_root, cs, start, ref_cache)
}

struct MacroSessionGuard;
impl Drop for MacroSessionGuard {
    fn drop(&mut self) {
        crate::languages::c_lang::macro_catalog::end_index_session();
    }
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
///   1.  Check for manifest changes — warn if package re-detection needed.
///   2.  Compute blast radius BEFORE any mutations (edges deleted by CASCADE).
///   3.  Delete removed files.
///   4.  Invalidate ref cache for changed files.
///   5.  Parse changed files (parallel via Rayon).
///   6.  Assign package_id to parsed files from DB packages table.
///   7.  Write files + symbols via shared pipeline.
///   8.  Update FTS content + code chunks.
///   9.  Load full symbol map (changed + unchanged).
///   10. Parse + re-resolve blast-radius affected files.
///   11. Cross-file resolution.
///   12. Store indexed_commit in metadata.
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

    // --- Step 1: Manifest change check (5b) ---
    // If any changed file is a package manifest, package detection is stale.
    // Re-run filesystem-only detection and rewrite the `packages` table so
    // downstream `package_id` assignment, resolver `ProjectContext`, and
    // workspace graph queries all see the new layout. This is the same
    // detection pass `full_index` runs at step 3b.
    const MANIFEST_NAMES: &[&str] = &[
        "package.json", "Cargo.toml", "go.mod", "pyproject.toml",
        "pubspec.yaml", "mix.exs", "Package.swift", "composer.json",
    ];
    let manifest_changed = changed_paths.iter().any(|p| {
        std::path::Path::new(p)
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| MANIFEST_NAMES.contains(&n))
            .unwrap_or(false)
    });

    // Load packages once and reuse for `assign_package_ids` AND the resolver's
    // `ProjectContext::initialize`. The full pipeline detects packages first,
    // writes them, then threads the vec through both writes and resolution;
    // incremental was previously dropping it on the floor at the resolver
    // step, which silently degraded monorepo-aware resolution after any
    // incremental save.
    let packages: Vec<crate::types::PackageInfo> = if manifest_changed {
        let (fresh, workspace_kind) =
            crate::indexer::stage_discover::detect_packages(project_root);
        let written = if fresh.is_empty() {
            // Manifest removed: clear the table so stale packages don't bleed
            // through. Stamping `package_id` to NULL for orphaned files is
            // out of scope for this branch — they'll re-stamp on the next
            // full reindex.
            warn!("Manifest change yielded zero packages; package rows not rewritten.");
            match write::load_packages_from_db(db) {
                Ok(p) => p,
                Err(e) => {
                    warn!("Failed to reload packages after empty detection: {e}");
                    Vec::new()
                }
            }
        } else {
            match write::write_packages(db, &fresh) {
                Ok(w) => {
                    info!("Manifest changed; rewrote {} workspace package(s)", w.len());
                    if let Some(kind) = workspace_kind {
                        if let Err(e) = changeset::set_meta(db, "workspace_kind", &kind) {
                            warn!("Failed to store workspace_kind: {e}");
                        }
                    }
                    w
                }
                Err(e) => {
                    warn!("Failed to rewrite packages after manifest change: {e}");
                    write::load_packages_from_db(db).unwrap_or_default()
                }
            }
        };
        written
    } else {
        match write::load_packages_from_db(db) {
            Ok(p) => p,
            Err(e) => {
                warn!("Failed to load packages from DB: {e}");
                Vec::new()
            }
        }
    };

    // --- Step 2: Blast radius (find dependents before CASCADE deletes edges) ---
    let dependent_paths = find_dependent_files(db, &changed_paths)?;
    if !dependent_paths.is_empty() {
        debug!(
            "Blast radius: {} files depend on changed files",
            dependent_paths.len()
        );
    }

    // --- Step 3: Delete removed files ---
    if !cs.deleted.is_empty() {
        write::delete_files(db, &cs.deleted)?;
    }

    // --- Step 4: Invalidate ref cache ---
    if let Some(rc) = ref_cache {
        let mut guard = rc.lock().unwrap();
        for path in &cs.deleted {
            guard.invalidate(path);
        }
        for w in cs.added.iter().chain(cs.modified.iter()) {
            guard.invalidate(&w.relative_path);
        }
    }

    // --- Step 5: Parse changed files (parallel) ---
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

    // --- Step 6: Assign package_id (5a) ---
    // Use the package list hoisted above (re-detected if a manifest changed,
    // else loaded fresh from DB) to stamp `package_id` on parsed files.
    if !parsed.is_empty() && !packages.is_empty() {
        write::assign_package_ids(&mut parsed, &packages);
    }

    if parsed.is_empty() && cs.deleted.is_empty() {
        stats.duration_ms = start.elapsed().as_millis() as u64;
        return Ok(stats);
    }

    // --- Step 7: Write files + symbols (shared pipeline) ---
    let (file_id_map, symbol_id_map) = if !parsed.is_empty() {
        let (fmap, smap) =
            write::write_parsed_files(db, &parsed).context("Failed to write index")?;
        stats.symbols_written = smap.len() as u32;
        (fmap, smap)
    } else {
        Default::default()
    };

    // --- Step 8: FTS + chunks (shared pipeline) ---
    if !parsed.is_empty() {
        write::update_fts_content(db, &parsed, &file_id_map)?;
        write::update_chunks(db, &parsed, &file_id_map, false)?;
    }

    // --- Step 9: (skipped) ---
    //
    // The previous step here ran `write::load_symbol_id_map` — a full
    // SELECT over every symbol in the DB just to build a (path, qname)
    // → id map for the heuristic resolver. On a 280k-symbol index that
    // alloced ~100MB per save. The same data now comes from the resolve
    // step's `SymbolIndex::augment_from_db_collecting_ids`, which folds
    // the project-wide map into the same SELECT it already runs to
    // populate the SymbolIndex. One DB scan instead of two.
    //
    // What we keep here in `symbol_id_map` is just the changed-file
    // entries from the write call — the heuristic gets full coverage
    // by merging this with `augmented_id_map` inside resolve.

    // --- Step 10: Blast radius re-resolution ---
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

    // --- Step 11: Cross-file resolution ---
    // Construct ProjectContext with the same `packages` vec used to stamp
    // `package_id` above. Resolution-time consumers (per-package manifest
    // lookup, declared-name workspace map, alias resolution) need this to
    // produce the same answers as the full pipeline; passing an empty slice
    // silently strips sibling-package resolution.
    let distinct_langs: HashSet<String> =
        parsed.iter().map(|pf| pf.language.clone()).collect();
    let mut project_ctx = super::project_context::ProjectContext::initialize(
        project_root,
        &packages,
        distinct_langs,
        crate::ecosystem::default_registry(),
    );

    // --- Step 11a: Plugin-owned cross-file state ---
    // Mirror the full-index Step 4b — populate the plugin state bag from
    // each active plugin. This runs on every incremental save, closing the
    // gap where Vue auto-imports and Robot library bindings were silently
    // absent for the incremental path.
    {
        let registry = languages::default_registry();
        let mut plugin_state = super::plugin_state::PluginStateBag::new();
        for plugin in registry.all() {
            if !project_ctx.language_presence.contains(plugin.id()) {
                continue;
            }
            plugin.populate_project_state(
                &mut plugin_state,
                &parsed,
                project_root,
                &project_ctx,
            );
        }
        project_ctx.plugin_state = plugin_state;
    }

    let rstats = resolve::resolve_and_write_incremental(db, &parsed, &symbol_id_map, Some(&project_ctx))
        .context("Failed to resolve references")?;
    stats.edges_written = rstats.resolved as u32;
    info!("Resolved {} edges for changed files", rstats.resolved);

    // --- Step 11b: Stage 3 — Connect + post-index enrichment ---
    //
    // Align with the full pipeline's 3-stage shape. Connector matching is
    // cross-file (a Start in changed file X may pair with a Stop in unchanged
    // file Y), so we re-run the full registry pass against the current DB
    // state + plugin-emitted points from the changed-file slice only. The
    // registry's dedupe drops overlap with points already stored from prior
    // runs.
    // One scan of the files table beats the previous N-query loop.
    // On a 50k-file index that's ~50k driver round-trips collapsed into
    // a single SELECT. The connector pass below needs the full map
    // (cross-file matches), so scoping the query is not an option.
    let file_id_map: std::collections::HashMap<String, i64> = {
        let conn = db.conn();
        let mut stmt = conn
            .prepare("SELECT path, id FROM files")
            .context("Failed to prepare files-id query")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .context("Failed to query files table")?;
        let mut map = std::collections::HashMap::new();
        for row in rows.flatten() {
            map.insert(row.0, row.1);
        }
        map
    };
    let plugin_points = crate::connectors::from_plugins::collect_plugin_connection_points(
        &parsed,
        &file_id_map,
        &symbol_id_map,
    );

    let mut resolved_plugin_points: Vec<crate::connectors::types::ConnectionPoint> = Vec::new();
    for plugin in registry.all() {
        let points = plugin.resolve_connection_points_incremental(
            db, project_root, &project_ctx, &changed_paths,
        );
        if !points.is_empty() {
            resolved_plugin_points.extend(points);
        }
    }

    let connector_registry = crate::connectors::registry::build_default_registry();
    // Incremental path: scope per-connector disk scans to changed +
    // dependent files. The C# DI / event-bus connectors override
    // `incremental_extract` to skip the full project sweep — saves
    // ~10k disk reads on a typical save event.
    if let Err(e) = connector_registry.run_incremental(
        db.conn(),
        project_root,
        &project_ctx,
        &plugin_points,
        &resolved_plugin_points,
        &changed_paths,
    ) {
        warn!("Incremental connector pass failed: {e}");
    }

    for plugin in registry.all() {
        plugin.post_index(db, project_root, &project_ctx);
    }

    // --- Step 12: Store indexed commit ---
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
