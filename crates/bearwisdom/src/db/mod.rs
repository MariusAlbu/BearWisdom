// =============================================================================
// db/mod.rs  —  Database connection wrapper
//
// The `Database` struct owns a rusqlite Connection and exposes the setup
// helpers.  All actual SQL lives in schema.rs (CREATE TABLE) and the
// various query/indexer modules (INSERT / SELECT).
// =============================================================================

pub mod schema;

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

/// Wraps a SQLite connection with the v2 schema applied.
pub struct Database {
    pub conn: Connection,
}

impl Database {
    /// Open (or create) a database file at `path`.
    ///
    /// # What happens on first open
    /// 1. Open the file (SQLite creates it if absent).
    /// 2. Apply WAL mode + performance PRAGMAs.
    /// 3. Create all tables and indexes (idempotent — IF NOT EXISTS).
    pub fn open(path: &Path) -> Result<Self> {
        let is_new = !path.exists();

        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open database at {}", path.display()))?;

        schema::apply_pragmas(&conn, is_new)
            .context("Failed to apply SQLite PRAGMAs")?;

        schema::create_schema(&conn)
            .context("Failed to create schema")?;

        Ok(Self { conn })
    }

    /// Open a database and attempt to load the sqlite-vec extension.
    ///
    /// Tries `SQLITE_VEC_PATH` env var for the extension library path.
    /// If the extension is unavailable, the database opens normally
    /// without vector search support (check with `has_vec_extension()`).
    pub fn open_with_vec(path: &Path) -> Result<Self> {
        let db = Self::open(path)?;
        if let Ok(vec_path) = std::env::var("SQLITE_VEC_PATH") {
            match db.try_load_vec_extension(&vec_path) {
                Ok(()) => tracing::info!("sqlite-vec loaded from {vec_path}"),
                Err(e) => tracing::warn!("sqlite-vec unavailable ({e}), vector search disabled"),
            }
        }
        Ok(db)
    }

    /// Returns true if the sqlite-vec extension is loaded and operational.
    pub fn has_vec_extension(&self) -> bool {
        self.conn
            .execute_batch(
                "CREATE VIRTUAL TABLE IF NOT EXISTS _vec_probe USING vec0(x float[1]);
                 DROP TABLE IF EXISTS _vec_probe;",
            )
            .is_ok()
    }

    fn try_load_vec_extension(&self, path: &str) -> Result<()> {
        // Safety: load_extension_enable/disable and load_extension are unsafe
        // because they can execute arbitrary native code. We control the path
        // via SQLITE_VEC_PATH env var and disable loading immediately after.
        unsafe {
            self.conn
                .load_extension_enable()
                .context("Failed to enable extension loading")?;
        }

        let result = unsafe { self.conn.load_extension(path, None) };

        // Always re-disable extension loading for security.
        let _ = self.conn.load_extension_disable();

        result.context("Failed to load vec0 extension")?;
        Ok(())
    }

    /// Open an in-memory database — used in unit tests.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .context("Failed to open in-memory database")?;

        schema::apply_pragmas(&conn, true)?;
        schema::create_schema(&conn)?;

        Ok(Self { conn })
    }
}
