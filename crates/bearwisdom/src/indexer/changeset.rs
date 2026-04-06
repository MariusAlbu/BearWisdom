// =============================================================================
// indexer/changeset.rs  —  unified change detection
//
// All change detection strategies produce a `ChangeSet` — the canonical input
// to the shared index pipeline.  The pipeline doesn't care how changes were
// detected; it only needs to know what files to add/modify/delete.
//
// Strategies:
//   • FullScan   — walk everything (first index or forced rebuild)
//   • GitDiff    — `git diff --name-status` between indexed commit and HEAD
//   • HashDiff   — walk + SHA-256 comparison (non-git repos)
//   • FileEvents — IDE/watcher-supplied change list
// =============================================================================

use crate::db::Database;
use crate::walker::{self, WalkedFile};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// ChangeSet — the unified output of all detection strategies
// ---------------------------------------------------------------------------

/// The set of file changes to process in one index pass.
#[derive(Debug, Default)]
pub struct ChangeSet {
    /// Files that are new (not previously indexed).
    pub added: Vec<WalkedFile>,
    /// Files whose content changed since last index.
    pub modified: Vec<WalkedFile>,
    /// Relative paths of files that were deleted.
    pub deleted: Vec<String>,
    /// Number of files that were unchanged (for stats reporting).
    pub unchanged: u32,
    /// Git HEAD commit at detection time (if available).
    pub commit: Option<String>,
}

impl ChangeSet {
    /// True if there are no changes to process.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.modified.is_empty() && self.deleted.is_empty()
    }

    /// Total number of files that need parsing (added + modified).
    pub fn files_to_parse(&self) -> Vec<&WalkedFile> {
        self.added.iter().chain(self.modified.iter()).collect()
    }

    /// Number of changed files (added + modified + deleted).
    pub fn change_count(&self) -> usize {
        self.added.len() + self.modified.len() + self.deleted.len()
    }
}

// ---------------------------------------------------------------------------
// Strategy: FullScan
// ---------------------------------------------------------------------------

/// Walk the entire project tree — every file is "added".
///
/// Used for the first index or a forced rebuild.  `pre_walked` allows callers
/// to supply an already-walked file list (e.g. from profile scanning) to skip
/// a redundant directory traversal.
pub fn full_scan(
    project_root: &Path,
    pre_walked: Option<Vec<WalkedFile>>,
) -> Result<ChangeSet> {
    let files = match pre_walked {
        Some(f) => {
            info!("Using pre-walked file list ({} files)", f.len());
            f
        }
        None => walker::walk(project_root)
            .with_context(|| format!("Failed to walk {}", project_root.display()))?,
    };

    info!("FullScan: {} files", files.len());

    Ok(ChangeSet {
        added: files,
        modified: Vec::new(),
        deleted: Vec::new(),
        unchanged: 0,
        commit: current_git_head(project_root),
    })
}

// ---------------------------------------------------------------------------
// Strategy: HashDiff
// ---------------------------------------------------------------------------

/// Walk the project tree and detect changes using mtime+size as a fast
/// pre-filter, falling back to SHA-256 hash comparison when metadata differs.
///
/// For unchanged files (mtime+size match), no file read or hash is needed.
/// This reduces I/O from O(all files) to O(changed files) for the common case.
///
/// Fallback for non-git repos.  For git repos, prefer `git_diff`.
pub fn hash_diff(db: &Database, project_root: &Path) -> Result<ChangeSet> {
    let files_on_disk = walker::walk(project_root)
        .with_context(|| format!("Failed to walk {}", project_root.display()))?;

    // Load existing file records from the database (including mtime + size).
    // (path -> (id, hash, mtime, size))
    let mut existing: HashMap<String, (i64, String, Option<i64>, Option<i64>)> = HashMap::new();
    {
        let mut stmt = db
            .prepare("SELECT id, path, hash, mtime, size FROM files")
            .context("Failed to query files")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            })
            .context("Failed to read files")?;
        for row in rows {
            let (id, path, hash, mtime, size) = row?;
            existing.insert(path, (id, hash, mtime, size));
        }
    }

    let mut changeset = ChangeSet::default();
    let mut seen_paths: HashSet<String> = HashSet::new();
    let mut skipped_by_mtime = 0u32;

    for walked in files_on_disk {
        seen_paths.insert(walked.relative_path.clone());

        match existing.get(&walked.relative_path) {
            Some((_id, _old_hash, Some(old_mtime), Some(old_size))) => {
                // Fast path: stat the file and compare mtime + size.
                // If both match, the content hasn't changed — skip the hash.
                let meta = std::fs::metadata(&walked.absolute_path);
                if let Ok(meta) = meta {
                    let disk_size = meta.len() as i64;
                    let disk_mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64);

                    if disk_size == *old_size && disk_mtime == Some(*old_mtime) {
                        changeset.unchanged += 1;
                        skipped_by_mtime += 1;
                        continue;
                    }
                }

                // mtime or size differs — fall through to hash check.
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

                if hash == _old_hash.as_str() {
                    changeset.unchanged += 1;
                } else {
                    changeset.modified.push(walked);
                }
            }
            Some((_id, old_hash, _, _)) => {
                // No mtime/size stored (pre-v0.3 database) — full hash check.
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

                if hash == old_hash.as_str() {
                    changeset.unchanged += 1;
                } else {
                    changeset.modified.push(walked);
                }
            }
            None => {
                // New file — no need to hash, it's definitely added.
                changeset.added.push(walked);
            }
        }
    }

    // Detect deleted files (in DB but not on disk).
    for (path, _) in &existing {
        if !seen_paths.contains(path) {
            changeset.deleted.push(path.clone());
        }
    }

    info!(
        "HashDiff: {} added, {} modified, {} deleted, {} unchanged ({} skipped by mtime)",
        changeset.added.len(),
        changeset.modified.len(),
        changeset.deleted.len(),
        changeset.unchanged,
        skipped_by_mtime,
    );

    Ok(changeset)
}

// ---------------------------------------------------------------------------
// Strategy: FileEvents (IDE / watcher)
// ---------------------------------------------------------------------------

/// Convert IDE/watcher file change events into a ChangeSet.
///
/// This is the fast path — no tree walk, no hashing.  The caller supplies
/// exactly which files changed and how.
pub fn from_file_events(
    project_root: &Path,
    changes: &[FileChangeEvent],
) -> Result<ChangeSet> {
    let mut changeset = ChangeSet::default();

    for change in changes {
        match change.change_kind {
            ChangeKind::Deleted => {
                changeset.deleted.push(change.relative_path.clone());
            }
            ChangeKind::Created | ChangeKind::Modified => {
                let abs_path = project_root.join(&change.relative_path);

                // Race: file deleted between watcher event and reindex.
                if !abs_path.exists() {
                    debug!(
                        "File no longer exists, skipping: {}",
                        change.relative_path
                    );
                    continue;
                }

                let language = match walker::detect_language(&abs_path) {
                    Some(l) => l,
                    None => continue,
                };

                let walked = WalkedFile {
                    relative_path: change.relative_path.clone(),
                    absolute_path: abs_path,
                    language,
                };

                match change.change_kind {
                    ChangeKind::Created => changeset.added.push(walked),
                    ChangeKind::Modified => changeset.modified.push(walked),
                    _ => unreachable!(),
                }
            }
        }
    }

    info!(
        "FileEvents: {} added, {} modified, {} deleted",
        changeset.added.len(),
        changeset.modified.len(),
        changeset.deleted.len()
    );

    Ok(changeset)
}

// ---------------------------------------------------------------------------
// Strategy: GitDiff
// ---------------------------------------------------------------------------

/// Use `git diff --name-status` to detect changes since the last indexed commit.
///
/// Falls back to `hash_diff` if:
///   - Not a git repository
///   - The indexed commit is unreachable (force push, rebase)
///   - git CLI is unavailable
///
/// This is the preferred strategy for subsequent reindexes in git repos —
/// it avoids reading and hashing every file.
pub fn git_diff(db: &Database, project_root: &Path) -> Result<ChangeSet> {
    // Read the last indexed commit from metadata.
    let indexed_commit = get_meta(db, "indexed_commit");

    let indexed_commit = match indexed_commit {
        Some(c) => c,
        None => {
            info!("GitDiff: no indexed_commit in metadata, falling back to HashDiff");
            return hash_diff(db, project_root);
        }
    };

    // Verify the commit is still reachable.
    let head = match current_git_head(project_root) {
        Some(h) => h,
        None => {
            info!("GitDiff: not a git repo or git unavailable, falling back to HashDiff");
            return hash_diff(db, project_root);
        }
    };

    if indexed_commit == head {
        info!("GitDiff: HEAD unchanged ({}), no changes", &head[..8]);
        return Ok(ChangeSet {
            commit: Some(head),
            ..Default::default()
        });
    }

    // Verify the indexed commit is reachable (handles force push / rebase).
    let reachable = std::process::Command::new("git")
        .args(["cat-file", "-t", &indexed_commit])
        .current_dir(project_root)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !reachable {
        warn!(
            "GitDiff: indexed commit {} is unreachable, falling back to HashDiff",
            &indexed_commit[..8]
        );
        return hash_diff(db, project_root);
    }

    // Run git diff --name-status.
    let output = std::process::Command::new("git")
        .args([
            "diff",
            "--name-status",
            "--no-renames",  // treat renames as delete + add for simplicity
            &indexed_commit,
            &head,
        ])
        .current_dir(project_root)
        .output()
        .context("Failed to run git diff")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!("GitDiff: git diff failed ({}), falling back to HashDiff", stderr.trim());
        return hash_diff(db, project_root);
    }

    let diff_output = String::from_utf8_lossy(&output.stdout);
    let mut changeset = ChangeSet {
        commit: Some(head),
        ..Default::default()
    };

    for line in diff_output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Format: "A\tpath" or "M\tpath" or "D\tpath"
        let (status, path) = match line.split_once('\t') {
            Some((s, p)) => (s, p),
            None => continue,
        };

        // Normalise path separators to forward slashes.
        let rel_path = path.replace('\\', "/");

        match status {
            "D" => {
                changeset.deleted.push(rel_path);
            }
            "A" | "M" | "T" => {
                let abs_path = project_root.join(&rel_path);
                if !abs_path.exists() {
                    continue;
                }
                let language = match walker::detect_language(&abs_path) {
                    Some(l) => l,
                    None => continue,
                };
                let walked = WalkedFile {
                    relative_path: rel_path,
                    absolute_path: abs_path,
                    language,
                };
                if status == "A" {
                    changeset.added.push(walked);
                } else {
                    changeset.modified.push(walked);
                }
            }
            _ => {
                debug!("GitDiff: ignoring unknown status '{}' for {}", status, rel_path);
            }
        }
    }

    info!(
        "GitDiff: {} added, {} modified, {} deleted ({}..{})",
        changeset.added.len(),
        changeset.modified.len(),
        changeset.deleted.len(),
        &indexed_commit[..8],
        changeset.commit.as_deref().map(|c| &c[..8]).unwrap_or("?"),
    );

    Ok(changeset)
}

// ---------------------------------------------------------------------------
// Watcher event types (re-exported from here for all consumers)
// ---------------------------------------------------------------------------

/// Describes a file change reported by a file watcher or IDE.
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
// Metadata helpers
// ---------------------------------------------------------------------------

/// Read a value from `_bearwisdom_meta`.
pub fn get_meta(db: &Database, key: &str) -> Option<String> {
    db.conn()
        .query_row(
            "SELECT value FROM _bearwisdom_meta WHERE key = ?1",
            [key],
            |r| r.get(0),
        )
        .ok()
}

/// Write a value to `_bearwisdom_meta` (upsert).
pub fn set_meta(db: &Database, key: &str, value: &str) -> Result<()> {
    db.conn().execute(
        "INSERT INTO _bearwisdom_meta (key, value)
         VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Git helpers
// ---------------------------------------------------------------------------

/// Get the current HEAD commit SHA, or None if not a git repo.
fn current_git_head(project_root: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(project_root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}
