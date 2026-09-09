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
pub(crate) mod lexical_visibility;
pub mod metrics;
mod migrations;
mod pool;
mod resolution_schema;
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
    std::fs::create_dir_all(&dir).with_context(|| {
        format!(
            "Cannot create .bearwisdom dir in {}",
            project_root.display()
        )
    })?;
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

        schema::apply_pragmas(&conn, is_new).context("Failed to apply SQLite PRAGMAs")?;

        schema::create_schema(&conn).context("Failed to create schema")?;

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
        let conn = Connection::open_in_memory().context("Failed to open in-memory database")?;

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

pub use pool::{DbPool, PoolGuard};
