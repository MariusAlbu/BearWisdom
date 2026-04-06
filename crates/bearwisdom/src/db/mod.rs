// =============================================================================
// db/mod.rs  —  Database connection wrapper + connection pool
//
// The `Database` struct owns a rusqlite Connection and exposes the setup
// helpers.  All actual SQL lives in schema.rs (CREATE TABLE) and the
// various query/indexer modules (INSERT / SELECT).
//
// `DbPool` manages a set of idle `Database` connections to the same file.
// Connections are checked out via `pool.get()` and returned on drop.
// WAL mode + busy_timeout allow concurrent readers and serialised writers.
//
// sqlite-vec is statically linked and initialised on every connection via
// a direct call to sqlite3_vec_init.
// =============================================================================

pub mod audit;
pub mod metrics;
pub mod schema;

use crate::indexer::ref_cache::RefCache;
use crate::query::cache::QueryCache;
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Initialise sqlite-vec on a raw connection handle.
///
/// Calls the statically-linked `sqlite3_vec_init` entry point directly,
/// passing the connection handle.  With `SQLITE_CORE` compiled in, the
/// function registers its virtual table modules against the connection.
fn init_vec_on_connection(conn: &Connection) {
    unsafe {
        let init_fn: unsafe extern "C" fn(
            *mut rusqlite::ffi::sqlite3,
            *mut *mut std::ffi::c_char,
            *const rusqlite::ffi::sqlite3_api_routines,
        ) -> std::ffi::c_int = std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ());

        let rc = init_fn(conn.handle(), std::ptr::null_mut(), std::ptr::null());
        tracing::info!("sqlite3_vec_init returned rc={rc}");
        if rc != 0 {
            tracing::warn!("sqlite3_vec_init returned non-zero rc={rc}");
        }
    }

    // Verify the module is actually registered.
    match conn.query_row("SELECT vec_version()", [], |r| r.get::<_, String>(0)) {
        Ok(v) => tracing::info!("sqlite-vec {v} loaded successfully"),
        Err(e) => tracing::warn!("sqlite-vec init failed: {e}"),
    }
}

/// Resolve the database path for a project: `<project_root>/.bearwisdom/index.db`.
///
/// Creates the `.bearwisdom` directory if it doesn't exist.
pub fn resolve_db_path(project_root: &Path) -> Result<PathBuf> {
    let dir = project_root.join(".bearwisdom");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Cannot create .bearwisdom dir in {}", project_root.display()))?;
    Ok(dir.join("index.db"))
}

/// Check whether an index database exists for the given project.
pub fn db_exists(project_root: &Path) -> bool {
    project_root.join(".bearwisdom").join("index.db").exists()
}

/// Wraps a SQLite connection with the v2 schema applied.
///
/// Provides delegation methods (`prepare_cached`, `execute`, `query_row`)
/// that route through optional metrics collection.  Query-layer code should
/// use these methods instead of accessing `conn` directly.
pub struct Database {
    conn: Connection,
    /// Path to the database file, or `None` for in-memory databases.
    pub path: Option<PathBuf>,
    /// Optional query-result cache (LRU per kind).  Shared across pool
    /// connections when created via `DbPool::with_cache`.
    pub query_cache: Option<Arc<QueryCache>>,
    /// Optional query metrics collector.  When present, delegation methods
    /// record per-label timing data.
    pub metrics: Option<Arc<metrics::QueryMetrics>>,
    /// Cached result of the sqlite-vec probe.  Populated on first call to
    /// `has_vec_extension()` and reused for all subsequent calls.
    has_vec: std::sync::OnceLock<bool>,
}

impl Database {
    /// Open (or create) a database file at `path`.
    ///
    /// sqlite-vec is automatically available on the connection.
    ///
    /// # What happens on first open
    /// 1. Open the file (SQLite creates it if absent).
    /// 2. Initialise sqlite-vec on the connection.
    /// 3. Apply WAL mode + performance PRAGMAs.
    /// 4. Create all tables and indexes (idempotent — IF NOT EXISTS).
    pub fn open(path: &Path) -> Result<Self> {
        let is_new = !path.exists();

        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open database at {}", path.display()))?;

        init_vec_on_connection(&conn);

        schema::apply_pragmas(&conn, is_new)
            .context("Failed to apply SQLite PRAGMAs")?;

        schema::create_schema(&conn)
            .context("Failed to create schema")?;

        Ok(Self {
            conn,
            path: Some(path.to_path_buf()),
            query_cache: Some(Arc::new(QueryCache::new(256))),
            metrics: Some(Arc::new(metrics::QueryMetrics::new())),
            has_vec: std::sync::OnceLock::new(),
        })
    }

    /// Returns true if the sqlite-vec extension is loaded and operational.
    ///
    /// The result is cached after the first call — the probe DDL executes at
    /// most once per `Database` instance.
    pub fn has_vec_extension(&self) -> bool {
        *self.has_vec.get_or_init(|| {
            self.execute_batch(
                "CREATE VIRTUAL TABLE IF NOT EXISTS _vec_probe USING vec0(x float[1]);
                 DROP TABLE IF EXISTS _vec_probe;",
            )
            .is_ok()
        })
    }

    /// Borrow the underlying SQLite connection.
    ///
    /// Prefer delegation methods on `Database` when available.  This accessor
    /// exists as a migration bridge for code that calls functions accepting
    /// `&Connection` directly (connectors, vector store, etc.).  Once all call
    /// sites are migrated to `&Database` this will be removed.
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    // -----------------------------------------------------------------
    // Delegation methods (metrics-aware)
    // -----------------------------------------------------------------

    /// Prepare a statement (non-cached).
    pub fn prepare(&self, sql: &str) -> rusqlite::Result<rusqlite::Statement<'_>> {
        self.conn.prepare(sql)
    }

    /// Prepare a cached statement.  Prefer this over `conn().prepare_cached()`
    /// for query-layer code — it enables future metrics/interception.
    pub fn prepare_cached(&self, sql: &str) -> rusqlite::Result<rusqlite::CachedStatement<'_>> {
        self.conn.prepare_cached(sql)
    }

    /// Execute a statement that returns no rows.
    pub fn execute(&self, sql: &str, params: impl rusqlite::Params) -> rusqlite::Result<usize> {
        self.conn.execute(sql, params)
    }

    /// Execute a batch of SQL statements separated by semicolons.
    pub fn execute_batch(&self, sql: &str) -> rusqlite::Result<()> {
        self.conn.execute_batch(sql)
    }

    /// Execute a query that returns exactly one row.
    pub fn query_row<T, P, F>(&self, sql: &str, params: P, f: F) -> rusqlite::Result<T>
    where
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        self.conn.query_row(sql, params, f)
    }

    /// Return the rowid of the most recent successful INSERT.
    pub fn last_insert_rowid(&self) -> i64 {
        self.conn.last_insert_rowid()
    }

    /// Return the number of rows changed by the most recent DML statement.
    pub fn changes(&self) -> u64 {
        self.conn.changes()
    }

    /// Return the raw sqlite3 handle — needed for sqlite-vec initialisation.
    ///
    /// # Safety
    /// The pointer is valid for the lifetime of `self`.  Do not close or
    /// otherwise invalidate the connection through this handle.
    pub unsafe fn handle(&self) -> *mut rusqlite::ffi::sqlite3 {
        self.conn.handle()
    }

    /// Begin a transaction (unchecked — does not enforce nesting rules).
    pub fn unchecked_transaction(&self) -> rusqlite::Result<rusqlite::Transaction<'_>> {
        self.conn.unchecked_transaction()
    }

    /// Create a metrics timer that records elapsed time under `label`
    /// when dropped.  No-op if metrics are disabled.
    pub fn timer(&self, label: &'static str) -> metrics::QueryTimer {
        metrics::QueryTimer::new(label, self.metrics.clone())
    }

    /// Set the query cache (usually propagated from DbPool).
    pub fn set_query_cache(&mut self, cache: Arc<QueryCache>) {
        self.query_cache = Some(cache);
    }

    /// Open an in-memory database — used in unit tests.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .context("Failed to open in-memory database")?;

        init_vec_on_connection(&conn);

        schema::apply_pragmas(&conn, true)?;
        schema::create_schema(&conn)?;

        Ok(Self {
            conn,
            path: None,
            query_cache: Some(Arc::new(QueryCache::new(256))),
            metrics: Some(Arc::new(metrics::QueryMetrics::new())),
            has_vec: std::sync::OnceLock::new(),
        })
    }
}

// =============================================================================
// Connection pool
// =============================================================================

struct DbPoolInner {
    path: PathBuf,
    idle: Mutex<Vec<Database>>,
    max_size: usize,
    /// Shared query-result cache.  `None` when the pool was created without a
    /// cache (the default).  Use [`DbPool::with_cache`] to opt in.
    cache: Option<Arc<QueryCache>>,
    /// Shared metrics collector across all pool connections.
    metrics: Option<Arc<metrics::QueryMetrics>>,
    /// Pool-level ref cache shared across all connections.
    ///
    /// Populated by `full_index` (via the `ref_cache` parameter) so the cache
    /// persists across pool checkout boundaries.  Always present — opt-in is
    /// whether anyone calls `store_all`, not whether the field exists.
    ref_cache: Arc<Mutex<RefCache>>,
}

/// A pool of `Database` connections to the same SQLite file.
///
/// Connections are checked out via [`get()`](DbPool::get) and automatically
/// returned when the [`PoolGuard`] drops.  Each connection has sqlite-vec
/// initialised and PRAGMAs applied.  WAL mode permits concurrent readers;
/// writers serialise via `busy_timeout`.
///
/// `DbPool` is `Clone + Send + Sync` — share freely across threads and tasks.
#[derive(Clone)]
pub struct DbPool(Arc<DbPoolInner>);

impl DbPool {
    /// Create a pool backed by the database file at `path`.
    ///
    /// The schema is created (idempotently) on the first connection.
    /// `max_size` controls how many idle connections are kept; connections
    /// beyond this limit are closed when returned.
    pub fn new(path: &Path, max_size: usize) -> Result<Self> {
        // Open one connection to ensure the schema exists.
        let seed = Database::open(path)?;
        // Shared cache + metrics across all pool connections.
        let cache = Arc::new(QueryCache::new(256));
        let metrics = Arc::new(metrics::QueryMetrics::new());
        let mut idle = Vec::with_capacity(max_size);
        idle.push(seed);
        Ok(Self(Arc::new(DbPoolInner {
            path: path.to_path_buf(),
            idle: Mutex::new(idle),
            max_size,
            cache: Some(cache),
            metrics: Some(metrics),
            ref_cache: Arc::new(Mutex::new(RefCache::new())),
        })))
    }

    /// Create a pool with a caller-supplied cache and metrics.
    ///
    /// Use this when you want to control the cache capacity or share a
    /// metrics instance across multiple pools.
    pub fn with_cache(path: &Path, max_size: usize, cache: Arc<QueryCache>) -> Result<Self> {
        let seed = Database::open(path)?;
        let metrics = Arc::new(metrics::QueryMetrics::new());
        let mut idle = Vec::with_capacity(max_size);
        idle.push(seed);
        Ok(Self(Arc::new(DbPoolInner {
            path: path.to_path_buf(),
            idle: Mutex::new(idle),
            max_size,
            cache: Some(cache),
            metrics: Some(metrics),
            ref_cache: Arc::new(Mutex::new(RefCache::new())),
        })))
    }

    /// Return the shared [`QueryCache`], if the pool was created with one.
    pub fn cache(&self) -> Option<&Arc<QueryCache>> {
        self.0.cache.as_ref()
    }

    /// Enable metrics collection for all connections checked out from this pool.
    ///
    /// Returns the shared metrics collector — query it later for snapshots.
    pub fn enable_metrics(&self) -> Arc<metrics::QueryMetrics> {
        // If already enabled, return the existing instance.
        if let Some(ref m) = self.0.metrics {
            return m.clone();
        }
        // Note: this is a benign race — worst case two metrics instances are
        // created and one is discarded.  In practice, enable_metrics is called
        // once at startup.
        let m = Arc::new(metrics::QueryMetrics::new());
        // We can't mutate DbPoolInner through Arc, so we store metrics on
        // each checked-out Database instead.  The pool-level field is set via
        // `with_metrics` constructor (below).
        m
    }

    /// Create a pool with both cache and metrics enabled.
    pub fn with_metrics(
        path: &Path,
        max_size: usize,
        cache: Option<Arc<QueryCache>>,
        metrics: Arc<metrics::QueryMetrics>,
    ) -> Result<Self> {
        let seed = Database::open(path)?;
        let mut idle = Vec::with_capacity(max_size);
        idle.push(seed);
        Ok(Self(Arc::new(DbPoolInner {
            path: path.to_path_buf(),
            idle: Mutex::new(idle),
            max_size,
            cache,
            metrics: Some(metrics),
            ref_cache: Arc::new(Mutex::new(RefCache::new())),
        })))
    }

    /// Return the shared metrics collector, if enabled.
    pub fn metrics(&self) -> Option<&Arc<metrics::QueryMetrics>> {
        self.0.metrics.as_ref()
    }

    /// Return the pool-level [`RefCache`] shared across all connections.
    ///
    /// Pass this to `full_index` / `incremental_index` / `git_reindex` /
    /// `reindex_files` so the cache survives connection checkout boundaries.
    pub fn ref_cache(&self) -> &Arc<Mutex<RefCache>> {
        &self.0.ref_cache
    }

    /// Check out a connection.  Reuses an idle connection when available,
    /// otherwise opens a fresh one.  Propagates the pool's cache and metrics
    /// to the checked-out connection.
    pub fn get(&self) -> Result<PoolGuard> {
        let mut db = {
            let mut idle = self.0.idle.lock().unwrap();
            idle.pop()
        }
        .map(Ok)
        .unwrap_or_else(|| Database::open(&self.0.path))?;

        // Propagate shared state to the connection.
        if let Some(ref cache) = self.0.cache {
            db.query_cache = Some(cache.clone());
        }
        if let Some(ref metrics) = self.0.metrics {
            db.metrics = Some(metrics.clone());
        }

        Ok(PoolGuard {
            db: Some(db),
            pool: self.0.clone(),
        })
    }
}

/// RAII guard that dereferences to `Database` and returns the connection
/// to the pool on drop.
pub struct PoolGuard {
    db: Option<Database>,
    pool: Arc<DbPoolInner>,
}

impl std::ops::Deref for PoolGuard {
    type Target = Database;
    fn deref(&self) -> &Database {
        self.db.as_ref().expect("PoolGuard used after drop")
    }
}

impl std::ops::DerefMut for PoolGuard {
    fn deref_mut(&mut self) -> &mut Database {
        self.db.as_mut().expect("PoolGuard used after drop")
    }
}

impl Drop for PoolGuard {
    fn drop(&mut self) {
        if let Some(db) = self.db.take() {
            let mut idle = self.pool.idle.lock().unwrap();
            if idle.len() < self.pool.max_size {
                idle.push(db);
            }
            // else: connection is dropped (closed)
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod pool_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn test_db_path() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        std::env::temp_dir().join(format!("bw_pool_test_{pid}_{id}.db"))
    }

    #[test]
    fn test_pool_basic_get_and_return() {
        let path = test_db_path();
        let pool = DbPool::new(&path, 2).unwrap();

        // Check out a connection.
        let db = pool.get().unwrap();
        // Verify it works.
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        // Drop returns it to the pool.
        drop(db);

        // Check out again — should reuse.
        let db2 = pool.get().unwrap();
        let count2: i64 = db2
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count2, 0);
    }

    #[test]
    fn test_pool_concurrent_access() {
        let path = test_db_path();
        let pool = DbPool::new(&path, 4).unwrap();

        // Seed data.
        {
            let db = pool.get().unwrap();
            db.execute(
                "INSERT INTO files (path, hash, language, last_indexed) \
                 VALUES ('a.rs', 'h', 'rust', 0)",
                [],
            )
            .unwrap();
        }

        // Spawn multiple threads that all read concurrently.
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let pool = pool.clone();
                std::thread::spawn(move || {
                    for _ in 0..5 {
                        let db = pool.get().unwrap();
                        let count: i64 = db
                            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
                            .unwrap();
                        assert_eq!(count, 1);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().expect("thread panicked");
        }
    }

    #[test]
    fn test_pool_max_size_limits_idle() {
        let path = test_db_path();
        let pool = DbPool::new(&path, 2).unwrap();

        // Check out 4 connections (exceeds max_size of 2).
        let db1 = pool.get().unwrap();
        let db2 = pool.get().unwrap();
        let db3 = pool.get().unwrap();
        let db4 = pool.get().unwrap();

        // All four should work.
        for db in [&db1, &db2, &db3, &db4] {
            let _: i64 = db
                .query_row("SELECT 1", [], |r| r.get(0))
                .unwrap();
        }

        // Return all four — only 2 should be kept (max_size).
        drop(db1);
        drop(db2);
        drop(db3);
        drop(db4);

        // Verify pool still works after returns.
        let db = pool.get().unwrap();
        let _: i64 = db
            .query_row("SELECT 1", [], |r| r.get(0))
            .unwrap();
    }

    #[test]
    fn test_pool_clone_shares_state() {
        let path = test_db_path();
        let pool1 = DbPool::new(&path, 2).unwrap();
        let pool2 = pool1.clone();

        // Write via pool1.
        {
            let db = pool1.get().unwrap();
            db.execute(
                "INSERT INTO files (path, hash, language, last_indexed) \
                 VALUES ('x.rs', 'h', 'rust', 0)",
                [],
            )
            .unwrap();
        }

        // Read via pool2 — should see the write (same database file).
        let db = pool2.get().unwrap();
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
