// =============================================================================
// db/mod.rs  —  Database connection wrapper
//
// The `Database` struct owns a rusqlite Connection and exposes the setup
// helpers.  All actual SQL lives in schema.rs (CREATE TABLE) and the
// various query/indexer modules (INSERT / SELECT).
//
// sqlite-vec is statically linked and initialised on every connection via
// a direct call to sqlite3_vec_init.
// =============================================================================

pub mod schema;

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

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
pub struct Database {
    pub conn: Connection,
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

        Ok(Self { conn })
    }

    /// Open a database with vector search support.
    ///
    /// This is now identical to `open()` since sqlite-vec is statically
    /// linked.  Kept for API compatibility — callers don't need to change.
    pub fn open_with_vec(path: &Path) -> Result<Self> {
        Self::open(path)
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

    /// Open an in-memory database — used in unit tests.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .context("Failed to open in-memory database")?;

        init_vec_on_connection(&conn);

        schema::apply_pragmas(&conn, true)?;
        schema::create_schema(&conn)?;

        Ok(Self { conn })
    }
}
