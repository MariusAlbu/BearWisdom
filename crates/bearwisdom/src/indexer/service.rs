// =============================================================================
// indexer/service.rs — IndexService: BW core's index-keeper
//
// Owns:
//   • a `DbPool` for query consumers (MCP, Web, CLI watch)
//   • optional `notify` file watcher that triggers `reindex_files` on edits
//
// Consumers (MCP, Web, CLI watch) call `IndexService::open(...)` and pull the
// pool out via `service.pool().clone()`. They never call indexing functions
// directly — that's BW core's responsibility, and the watcher keeps the
// SQLite graph in sync with the working tree without consumer involvement.
//
// The single-pass startup-only reindex pattern that previously lived in
// `bearwisdom-mcp/src/main.rs` and the inline notify loop in
// `bearwisdom-cli/src/main.rs:cmd_watch` are both subsumed by this module.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use rustc_hash::FxHashSet;
use tracing::{debug, info, warn};

use crate::db::DbPool;
use crate::indexer::changeset::{self, ChangeKind, FileChangeEvent};
use crate::indexer::full::full_index;
use crate::indexer::incremental::{git_reindex, incremental_index, reindex_files, IncrementalStats};
use crate::types::IndexStats;

/// Source-file extensions watched and forwarded to the incremental indexer.
/// Mirrors the supported language set; non-source files (e.g. binary assets,
/// build artifacts) are filtered out before triggering a reindex.
const SOURCE_EXTENSIONS: &[&str] = &[
    "cs", "ts", "tsx", "js", "jsx", "rs", "py", "go", "java", "rb", "php",
    "kt", "swift", "scala", "dart", "ex", "exs", "c", "h", "cpp", "hpp",
    "sh", "bash", "html", "css", "scss", "json", "yaml", "yml", "xml",
    "sql", "toml", "md", "lua", "r", "hs", "proto",
];

/// Configuration for `IndexService::open`.
#[derive(Clone, Debug)]
pub struct IndexServiceOptions {
    /// Number of pooled SQLite connections.
    pub pool_size: usize,
    /// If true, spawn a `notify` watcher that triggers `reindex_files` on
    /// source-file changes within the project root.
    pub watch: bool,
    /// Debounce window for batched watcher events.
    pub debounce: Duration,
}

impl Default for IndexServiceOptions {
    fn default() -> Self {
        Self {
            pool_size: 4,
            watch: true,
            debounce: Duration::from_millis(250),
        }
    }
}

/// Long-lived index service. Owns the pool and the file watcher.
///
/// Construction is cheap: `open` opens (or creates) the SQLite DB and starts
/// the watcher (if enabled). It does NOT run an initial reindex —
/// `reindex_now` is a separate method so callers can choose whether to block
/// startup on the initial pass or run it in the background.
pub struct IndexService {
    pool: DbPool,
    project_root: PathBuf,
    /// Watcher handle. Drop stops the watcher and joins the worker thread.
    _watcher: Option<WatcherHandle>,
}

/// Outcome of a `reindex_now` call. Carries the strategy chosen and the
/// underlying stats for observability.
#[derive(Debug)]
pub enum ReindexStats {
    Full(IndexStats),
    Incremental(IncrementalStats),
}

impl IndexService {
    /// Open the index at `db_path` for the project at `project_root`.
    ///
    /// Starts the file watcher per `opts`. Does not run an initial reindex —
    /// the caller should invoke `reindex_now` (synchronously or on a background
    /// thread) if it wants the index brought up to current state.
    pub fn open(
        db_path: &Path,
        project_root: &Path,
        opts: IndexServiceOptions,
    ) -> Result<Self> {
        let pool = DbPool::new(db_path, opts.pool_size)
            .with_context(|| format!("create pool for {}", db_path.display()))?;
        let project_root = project_root.to_path_buf();
        let watcher = if opts.watch {
            Some(spawn_watcher(pool.clone(), project_root.clone(), opts.debounce)?)
        } else {
            None
        };
        Ok(Self {
            pool,
            project_root,
            _watcher: watcher,
        })
    }

    /// Pool of SQLite connections shared with query consumers.
    pub fn pool(&self) -> &DbPool {
        &self.pool
    }

    /// The project root this service indexes.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Synchronously bring the index to current working-tree state.
    ///
    /// Strategy:
    ///   1. Existing DB with `indexed_commit` meta → git-incremental.
    ///   2. Existing DB with files but no commit metadata → hash-incremental.
    ///   3. Empty DB → full index.
    pub fn reindex_now(&self) -> Result<ReindexStats> {
        let ref_cache = self.pool.ref_cache().clone();
        let mut db = self
            .pool
            .get()
            .map_err(|e| anyhow::anyhow!("pool acquire: {e}"))?;

        let prior_commit = changeset::get_meta(&db, "indexed_commit");
        if prior_commit.is_some() {
            let inc = git_reindex(&mut db, &self.project_root, Some(&ref_cache))?;
            return Ok(ReindexStats::Incremental(inc));
        }
        let file_count: i64 = db
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap_or(0);
        if file_count > 0 {
            let inc = incremental_index(&mut db, &self.project_root, Some(&ref_cache))?;
            return Ok(ReindexStats::Incremental(inc));
        }
        let stats = full_index(&mut db, &self.project_root, None, None, Some(&ref_cache))?;
        Ok(ReindexStats::Full(stats))
    }
}

// ---------------------------------------------------------------------------
// Watcher implementation (private)
// ---------------------------------------------------------------------------

/// Owns the `notify` watcher and the worker-thread join handle. Dropping
/// stops the watcher (closes the event channel) and joins the worker.
struct WatcherHandle {
    /// `Option` so `Drop` can take the watcher out and drop it before join.
    watcher: Option<RecommendedWatcher>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        // Drop the watcher first to close the event channel; the worker
        // thread then exits on `recv` returning Err(Disconnected).
        self.watcher.take();
        if let Some(h) = self.join_handle.take() {
            let _ = h.join();
        }
    }
}

fn spawn_watcher(
    pool: DbPool,
    project_root: PathBuf,
    debounce: Duration,
) -> Result<WatcherHandle> {
    let (event_tx, event_rx) = mpsc::channel::<Event>();

    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                let _ = event_tx.send(event);
            }
        },
        Config::default(),
    )
    .context("create file watcher")?;

    watcher
        .watch(&project_root, RecursiveMode::Recursive)
        .with_context(|| format!("watch {}", project_root.display()))?;

    info!(
        "IndexService: watching {} (debounce={}ms)",
        project_root.display(),
        debounce.as_millis()
    );

    let join_handle = thread::Builder::new()
        .name("bw-index-watcher".into())
        .spawn(move || run_watcher_loop(event_rx, pool, project_root, debounce))
        .context("spawn watcher thread")?;

    Ok(WatcherHandle {
        watcher: Some(watcher),
        join_handle: Some(join_handle),
    })
}

fn run_watcher_loop(
    event_rx: mpsc::Receiver<Event>,
    pool: DbPool,
    project_root: PathBuf,
    debounce: Duration,
) {
    let source_exts: FxHashSet<&str> = SOURCE_EXTENSIONS.iter().copied().collect();

    loop {
        // Block on first event. Channel disconnected = service dropped.
        let first = match event_rx.recv() {
            Ok(e) => e,
            Err(_) => break,
        };

        // Drain debounce window.
        let mut events = vec![first];
        let deadline = Instant::now() + debounce;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match event_rx.recv_timeout(remaining) {
                Ok(e) => events.push(e),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }

        // Convert + dedupe by relative path.
        let mut seen: FxHashSet<String> = FxHashSet::default();
        let mut changes: Vec<FileChangeEvent> = Vec::new();
        for event in &events {
            let change_kind = match event.kind {
                EventKind::Create(_) => ChangeKind::Created,
                EventKind::Modify(_) => ChangeKind::Modified,
                EventKind::Remove(_) => ChangeKind::Deleted,
                _ => continue,
            };
            for path in &event.paths {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if !source_exts.contains(ext) {
                    continue;
                }
                let rel = match path.strip_prefix(&project_root) {
                    Ok(r) => r.to_string_lossy().replace('\\', "/"),
                    Err(_) => continue,
                };
                if seen.insert(rel.clone()) {
                    changes.push(FileChangeEvent {
                        relative_path: rel,
                        change_kind,
                    });
                }
            }
        }
        if changes.is_empty() {
            continue;
        }

        let ref_cache = pool.ref_cache().clone();
        let mut db = match pool.get() {
            Ok(g) => g,
            Err(e) => {
                warn!("IndexService watcher: pool acquire failed: {e}");
                continue;
            }
        };
        match reindex_files(&mut db, &project_root, &changes, Some(&ref_cache)) {
            Ok(stats) => debug!(
                "IndexService watcher: reindexed +{} ~{} -{} files in {}ms",
                stats.files_added,
                stats.files_modified,
                stats.files_deleted,
                stats.duration_ms,
            ),
            Err(e) => warn!("IndexService watcher: reindex error: {e:#}"),
        }
    }
}
