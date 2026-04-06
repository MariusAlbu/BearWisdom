use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};

pub fn resolve_db_path(project_root: &Path) -> Result<PathBuf> {
    bearwisdom::resolve_db_path(project_root)
}

pub fn db_exists(project_root: &Path) -> bool {
    bearwisdom::db_exists(project_root)
}

// ---------------------------------------------------------------------------
// Pooled access (shared across all request handlers)
// ---------------------------------------------------------------------------

/// Shared connection pool state.  Holds a pool for the currently-active
/// project.  Created on first `POST /api/index`, reused by all GET handlers.
///
/// The web server typically serves one project at a time (the user POSTs
/// `/api/index` with a path, then all GETs use `?path=...`).  When a
/// different project is requested, a new pool replaces the old one.
pub struct PoolState {
    inner: Mutex<Option<(PathBuf, bearwisdom::DbPool)>>,
}

impl PoolState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    /// Get a pooled connection for the given project root.
    ///
    /// If the pool exists and matches the project, returns a connection.
    /// If the project differs or no pool exists, creates a new pool
    /// (auto-creates the DB if it doesn't exist — `Database::open` is
    /// idempotent).
    pub fn get_db(&self, project_root: &Path) -> Result<bearwisdom::PoolGuard> {
        let mut guard = self.inner.lock().unwrap();

        // Check if we already have a pool for this project.
        if let Some((ref path, ref pool)) = *guard {
            if path == project_root {
                return pool.get().context("Failed to get pooled connection");
            }
        }

        // Different project or first call — create a new pool.
        let db_path = resolve_db_path(project_root)?;
        if !db_path.exists() {
            anyhow::bail!(
                "No index found for {}. POST /api/index first.",
                project_root.display()
            );
        }

        let pool = bearwisdom::DbPool::new(&db_path, 4)
            .with_context(|| format!("Failed to create pool for {}", db_path.display()))?;
        let conn = pool.get().context("Failed to get pooled connection")?;
        *guard = Some((project_root.to_path_buf(), pool));
        Ok(conn)
    }

    /// Create or replace the pool for a project (called after indexing).
    pub fn set_pool(&self, project_root: &Path, pool: bearwisdom::DbPool) {
        let mut guard = self.inner.lock().unwrap();
        *guard = Some((project_root.to_path_buf(), pool));
    }
}
