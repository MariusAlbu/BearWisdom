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

#[cfg(test)]
#[path = "changeset_tests.rs"]
mod tests;
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
    let mut files = match pre_walked {
        Some(f) => {
            info!("Using pre-walked file list ({} files)", f.len());
            f
        }
        None => walker::walk(project_root)
            .with_context(|| format!("Failed to walk {}", project_root.display()))?,
    };

    // Secondary pass: pull files in gitignored directories that the project's
    // own source explicitly imports from. The canonical case is generated
    // client code (Prisma, GraphQL codegen, OpenAPI) where the output dir is
    // gitignored by convention but the project source imports from it.
    // Source imports are an authoritative signal; without this pass those
    // imports go unresolved despite the target being one filesystem read away.
    let extra = super::secondary_scan::pull_gitignored_imports(project_root, &files);
    if !extra.is_empty() {
        info!(
            "FullScan: pulled {} additional file(s) from gitignored paths referenced by source imports",
            extra.len()
        );
        files.extend(extra);
        files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    }

    // TU allowlist filter: when the project has a `compile_commands.json`,
    // CMake or Bear has already enumerated exactly which translation
    // units the build compiles. Drop any walked C/C++ source file that
    // isn't in the TU set — those are platform-conditional or missing-
    // dep files (echo-servers/poco_echo.cpp without Poco installed,
    // ssl/gnutls.c on Windows, event/io_uring.c off Linux) the build
    // already chose to skip. Headers and non-C/C++ files always pass
    // through; they aren't TUs and aren't listed in compile_commands.
    //
    // Sanity cap: if the filter would drop more than ~20% of the
    // project's C/C++ source files, the manifest is too incomplete to
    // trust as ground truth. Common cause: example/demo subprojects
    // that each need a different optional dep (clay's renderers/
    // examples — Cairo, raylib, SDL2, sokol, termbox2 — only the
    // renderers whose deps are installed end up in the build).
    // Dropping that many files removes the project's core
    // demonstration code along with the genuinely-conditional
    // outliers. Keep them all in that case; the resolver still
    // handles unresolved refs gracefully, and the user sees all of
    // their project rather than a build-config-determined slice.
    if let Some(tu_set) = crate::ecosystem::compile_commands::tu_file_set(project_root) {
        let total_sources = files.iter().filter(|w| is_c_or_cpp_source(&w.relative_path)).count();
        let proposed_drops: Vec<usize> = files
            .iter()
            .enumerate()
            .filter(|(_, w)| {
                if !is_c_or_cpp_source(&w.relative_path) { return false }
                let canonical = w
                    .absolute_path
                    .canonicalize()
                    .unwrap_or_else(|_| w.absolute_path.clone());
                !tu_set.contains(&canonical)
            })
            .map(|(idx, _)| idx)
            .collect();
        let drop_ratio = if total_sources == 0 {
            0.0
        } else {
            proposed_drops.len() as f64 / total_sources as f64
        };
        // 50% threshold: drop ratios above this signal a build manifest
        // that excludes major portions of the project (clay: 66% drop,
        // because its example renderers each need a different optional
        // dep and only one set is configured at a time). Below 50%,
        // dropped files are typically genuinely-conditional outliers
        // (libhv at 39% drops echo-servers comparison TUs that need
        // Poco/asio/grpc; keepassxc at 29% drops platform-conditional
        // macOS Carbon and libusb code).
        const MAX_DROP_RATIO: f64 = 0.50;
        if drop_ratio > MAX_DROP_RATIO {
            warn!(
                "FullScan: TU allowlist would drop {}/{} ({:.0}%) C/C++ source files — manifest looks incomplete; keeping all walked sources",
                proposed_drops.len(), total_sources, drop_ratio * 100.0
            );
        } else if !proposed_drops.is_empty() {
            // Apply drops. Convert indices to a HashSet for O(1) retain.
            let drop_set: HashSet<usize> = proposed_drops.iter().copied().collect();
            let dropped = drop_set.len();
            files = files
                .into_iter()
                .enumerate()
                .filter_map(|(idx, w)| if drop_set.contains(&idx) { None } else { Some(w) })
                .collect();
            info!(
                "FullScan: TU allowlist dropped {} C/C++ source file(s) absent from compile_commands.json",
                dropped
            );
        }
    }

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
        // Only internal (project-tree) files participate in changeset diffing.
        // External-origin rows (from stdlib + package ecosystems) live
        // outside the project tree and are re-discovered/re-walked on every
        // full index; they must not appear as "deleted" here just because
        // their path isn't found under project_root.
        let mut stmt = db
            .prepare("SELECT id, path, hash, mtime, size FROM files WHERE origin = 'internal'")
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

    let mut changeset = ChangeSet {
        commit: Some(head.clone()),
        ..Default::default()
    };

    // Commit-range diff: catches files that changed between the indexed
    // commit and HEAD. Skipped when commits match — there are no committed
    // changes to find. The working-tree pass below still runs.
    if indexed_commit != head {
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

        let output = std::process::Command::new("git")
            .args([
                "diff",
                "--name-status",
                "--no-renames",
                &indexed_commit,
                &head,
            ])
            .current_dir(project_root)
            .output()
            .context("Failed to run git diff")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(
                "GitDiff: commit-range diff failed ({}), falling back to HashDiff",
                stderr.trim()
            );
            return hash_diff(db, project_root);
        }

        let diff_output = String::from_utf8_lossy(&output.stdout);
        for line in diff_output.lines() {
            apply_diff_line(line, project_root, &mut changeset);
        }
    }

    // Working-tree pass: catches uncommitted modifications, staged changes,
    // and untracked files. Without this pass an `indexed_commit == HEAD`
    // situation with mid-flight working changes would produce an empty
    // ChangeSet and modified files would never get re-extracted.
    apply_working_tree_changes(project_root, &mut changeset)?;

    deduplicate_changeset(&mut changeset);

    info!(
        "GitDiff: {} added, {} modified, {} deleted ({}..{}) + working tree",
        changeset.added.len(),
        changeset.modified.len(),
        changeset.deleted.len(),
        &indexed_commit[..8.min(indexed_commit.len())],
        changeset
            .commit
            .as_deref()
            .map(|c| &c[..8.min(c.len())])
            .unwrap_or("?"),
    );

    Ok(changeset)
}

/// Parse one `git diff --name-status` output line ("M\tpath", "A\tpath", …)
/// and apply it to the changeset. Used by both the commit-range diff and
/// the working-tree diff (`git diff HEAD`).
fn apply_diff_line(line: &str, project_root: &Path, changeset: &mut ChangeSet) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    let (status, path) = match line.split_once('\t') {
        Some((s, p)) => (s, p),
        None => return,
    };
    let rel_path = path.replace('\\', "/");

    match status {
        "D" => changeset.deleted.push(rel_path),
        "A" | "M" | "T" => {
            let abs_path = project_root.join(&rel_path);
            if !abs_path.exists() {
                return;
            }
            let language = match walker::detect_language(&abs_path) {
                Some(l) => l,
                None => return,
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
            debug!(
                "GitDiff: ignoring unknown status '{}' for {}",
                status, rel_path
            );
        }
    }
}

/// Add working-tree changes to the changeset:
///   1. Tracked files modified or deleted vs HEAD (`git diff --name-status HEAD`).
///   2. Untracked files honoring .gitignore (`git ls-files --others --exclude-standard`).
fn apply_working_tree_changes(
    project_root: &Path,
    changeset: &mut ChangeSet,
) -> Result<()> {
    // Tracked working-tree changes.
    let diff_output = std::process::Command::new("git")
        .args(["diff", "--name-status", "--no-renames", "HEAD"])
        .current_dir(project_root)
        .output()
        .context("Failed to run git diff HEAD")?;

    if diff_output.status.success() {
        let text = String::from_utf8_lossy(&diff_output.stdout);
        for line in text.lines() {
            apply_diff_line(line, project_root, changeset);
        }
    } else {
        let stderr = String::from_utf8_lossy(&diff_output.stderr);
        debug!(
            "GitDiff: working-tree diff failed ({}); skipping uncommitted edits",
            stderr.trim()
        );
    }

    // Untracked files. `--exclude-standard` honors .gitignore, .git/info/exclude,
    // and the user's global excludesFile so generated/derived files don't get
    // pulled in.
    let untracked_output = std::process::Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .current_dir(project_root)
        .output()
        .context("Failed to run git ls-files --others")?;

    if untracked_output.status.success() {
        let text = String::from_utf8_lossy(&untracked_output.stdout);
        for raw in text.lines() {
            let rel_path = raw.trim().replace('\\', "/");
            if rel_path.is_empty() {
                continue;
            }
            let abs_path = project_root.join(&rel_path);
            if !abs_path.exists() {
                continue;
            }
            let language = match walker::detect_language(&abs_path) {
                Some(l) => l,
                None => continue,
            };
            changeset.added.push(WalkedFile {
                relative_path: rel_path,
                absolute_path: abs_path,
                language,
            });
        }
    } else {
        let stderr = String::from_utf8_lossy(&untracked_output.stderr);
        debug!(
            "GitDiff: untracked listing failed ({}); skipping new files",
            stderr.trim()
        );
    }

    Ok(())
}

/// Collapse duplicate entries that can arise from union-merging the
/// commit-range and working-tree passes. Working-tree state wins:
///   * present in `added` and `modified`         → keep `modified`
///   * present in `deleted` and (added|modified) → keep the live entry
///   * duplicate within a single bucket          → keep the first
fn deduplicate_changeset(cs: &mut ChangeSet) {
    use std::collections::HashSet;

    // Pass 1: dedupe within each bucket.
    let mut seen: HashSet<String> = HashSet::new();
    cs.added.retain(|w| seen.insert(w.relative_path.clone()));
    seen.clear();
    cs.modified.retain(|w| seen.insert(w.relative_path.clone()));
    seen.clear();
    cs.deleted.retain(|p| seen.insert(p.clone()));

    // Pass 2: when a path appears in both `added` and `modified`, prefer
    // `modified` (working-tree edit on top of a committed add).
    let modified_paths: HashSet<String> =
        cs.modified.iter().map(|w| w.relative_path.clone()).collect();
    cs.added.retain(|w| !modified_paths.contains(&w.relative_path));

    // Pass 3: a live add/mod always supersedes a stale delete.
    let live_paths: HashSet<String> = cs
        .added
        .iter()
        .chain(cs.modified.iter())
        .map(|w| w.relative_path.clone())
        .collect();
    cs.deleted.retain(|p| !live_paths.contains(p));
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
fn is_c_or_cpp_source(relative_path: &str) -> bool {
    // Match TU file extensions only — headers (.h/.hpp/.hxx) are not
    // listed in compile_commands.json and must pass through the
    // allowlist filter unchanged.
    let lower = relative_path.to_ascii_lowercase();
    lower.ends_with(".c")
        || lower.ends_with(".cc")
        || lower.ends_with(".cpp")
        || lower.ends_with(".cxx")
        || lower.ends_with(".c++")
        || lower.ends_with(".m")
        || lower.ends_with(".mm")
}

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
