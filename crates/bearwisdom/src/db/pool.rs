// =============================================================================
// db/pool.rs — connection pool over Database
//
// Connections are checked out via DbPool::get and returned when the PoolGuard
// drops. Each connection has sqlite-vec initialised and PRAGMAs applied; WAL
// permits concurrent readers while writers serialise via busy_timeout.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;

use crate::indexer::ref_cache::RefCache;
use crate::query::cache::QueryCache;

use super::{Database, metrics};

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
            let mut idle = self.0.idle.lock().unwrap_or_else(|e| e.into_inner());
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
            db: std::mem::ManuallyDrop::new(db),
            pool: self.0.clone(),
        })
    }
}

/// RAII guard that dereferences to `Database` and returns the connection
/// to the pool on drop.
pub struct PoolGuard {
    db: std::mem::ManuallyDrop<Database>,
    pool: Arc<DbPoolInner>,
}

impl std::ops::Deref for PoolGuard {
    type Target = Database;
    fn deref(&self) -> &Database {
        &self.db
    }
}

impl std::ops::DerefMut for PoolGuard {
    fn deref_mut(&mut self) -> &mut Database {
        &mut self.db
    }
}

impl Drop for PoolGuard {
    fn drop(&mut self) {
        // SAFETY: `self.db` is valid — ManuallyDrop prevents the inner drop,
        // so we own the value here and can take it exactly once (on drop).
        let db = unsafe { std::mem::ManuallyDrop::take(&mut self.db) };
        let mut idle = self.pool.idle.lock().unwrap_or_else(|e| e.into_inner());
        if idle.len() < self.pool.max_size {
            idle.push(db);
        }
        // else: connection falls out of scope here and is closed
    }
}

#[cfg(test)]
#[path = "pool_tests.rs"]
mod pool_tests;
