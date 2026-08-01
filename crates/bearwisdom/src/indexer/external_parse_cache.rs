// =============================================================================
// indexer/external_parse_cache.rs — persistent parse cache for external files
//
// The AssemblyMetadata analog (Roslyn): an external source file is parsed at
// most once per machine. The cache is content-addressed — keyed by
// (extractor_schema_version, absolute_path, sha-256 content_hash) — so a fresh
// reindex, a `--force`, or a different project sharing the same dependency
// skips tree-sitter + the extraction walk (the expensive half) and rebuilds the
// ParsedFile from the stored extraction.
//
// What is cached: the full extraction the external ingest/write surface reads
// — symbols with their type surface (declared/return/param types, generic
// params, carried structurally via `external_parse_payload` so arena ids
// re-intern into the current run's `TypeArena`), refs with their chains,
// routes, origin-language and snippet flags, alias targets, and component
// selectors. A cache-hit rehydration must resolve identically to the fresh
// parse that produced it. NOT cached: `content` (bulk — consumers re-read the
// file from disk) and `mtime` (machine state — recomputed from fs metadata on
// rehydration, exactly as the fresh parse path does).
//
// Best-effort: any open/read/write error disables the cache for that op — a
// missing or unwritable cache only costs a re-parse, never correctness.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use once_cell::sync::Lazy;
use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::external_parse_payload::CachedParse;
use crate::type_checker::core::types::TypeArena;
use crate::types::ParsedFile;

/// Bumped whenever the cached extraction shape changes. It is part of the key,
/// so a bump makes every prior entry un-matchable (effectively a full flush).
const EXTRACTOR_SCHEMA_VERSION: u32 = 21;

#[cfg(test)]
#[path = "external_parse_cache_tests.rs"]
mod tests;

static CACHE: Lazy<Option<Mutex<Connection>>> = Lazy::new(open_cache);

fn open_cache() -> Option<Mutex<Connection>> {
    let dir = std::env::var_os("BEARWISDOM_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|d| d.join("bearwisdom")))?;
    std::fs::create_dir_all(&dir).ok()?;
    let conn = Connection::open(dir.join("externals.db")).ok()?;
    // WAL via pragma_update — `execute_batch`/`execute` reject `PRAGMA
    // journal_mode=WAL` because it returns a row. Best-effort: a failed WAL
    // switch just leaves the default rollback journal.
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    conn.execute(
        "CREATE TABLE IF NOT EXISTS parse_cache (key TEXT PRIMARY KEY, payload TEXT NOT NULL)",
        [],
    )
    .ok()?;
    Some(Mutex::new(conn))
}

fn cache_key(abs_path: &Path, content_hash: &str) -> String {
    format!(
        "{EXTRACTOR_SCHEMA_VERSION}:{}:{content_hash}",
        normalize_path_key(abs_path)
    )
}

/// Canonical, route-independent string for a path used as a cache key. The same
/// logical external file can arrive with different separators (`\` vs `/`) and
/// unfolded `.`/`..` segments depending on which locate / re-export route built
/// the `PathBuf` (`base.join("../dist/./x.d.ts")`). Without normalization the
/// same file keys under several strings, so a warm cache accumulates duplicate
/// rows across runs/binaries and a fresh run materializes a different file set.
/// Fold `.`/`..` and emit `/`-separated segments so one file maps to one key —
/// purely string-based (no filesystem access, works for non-existent paths and
/// resolves no symlinks).
fn normalize_path_key(abs_path: &Path) -> String {
    use std::path::Component;
    let mut prefix = String::new();
    let mut rooted = false;
    let mut segs: Vec<String> = Vec::new();
    for comp in abs_path.components() {
        match comp {
            Component::Prefix(p) => prefix = p.as_os_str().to_string_lossy().replace('\\', "/"),
            Component::RootDir => rooted = true,
            Component::CurDir => {}
            Component::ParentDir => {
                segs.pop();
            }
            Component::Normal(c) => segs.push(c.to_string_lossy().to_string()),
        }
    }
    let body = segs.join("/");
    match (prefix.is_empty(), rooted) {
        (false, true) => format!("{prefix}/{body}"),
        (false, false) => format!("{prefix}{body}"),
        (true, true) => format!("/{body}"),
        (true, false) => body,
    }
}

/// SHA-256 hex of file bytes — the same content hash `parse_file` records.
pub fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// File modification time (seconds since epoch) — the same derivation the
/// fresh parse path records, so a rehydrated file carries the same value.
fn fs_mtime(abs_path: &Path) -> Option<i64> {
    std::fs::metadata(abs_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
}

/// Rebuild a `ParsedFile` from a cache hit, or `None` on miss/error. The
/// `virtual_path` and `size` come from the caller (they are not part of the
/// content-addressed payload); cached types re-intern into `arena`.
pub fn get(
    abs_path: &Path,
    content_hash: &str,
    virtual_path: &str,
    size: u64,
    arena: &TypeArena,
) -> Option<ParsedFile> {
    let lock = CACHE.as_ref()?;
    let payload: String = {
        let conn = lock.lock().ok()?;
        conn.query_row(
            "SELECT payload FROM parse_cache WHERE key = ?1",
            [cache_key(abs_path, content_hash)],
            |r| r.get(0),
        )
        .ok()?
    };
    let cp: CachedParse = serde_json::from_str(&payload).ok()?;
    Some(cp.into_parsed(arena, virtual_path, content_hash, size, fs_mtime(abs_path)))
}

/// Store a freshly-parsed external file. Best-effort; errors are ignored.
pub fn put(abs_path: &Path, content_hash: &str, pf: &ParsedFile, arena: &TypeArena) {
    let Some(lock) = CACHE.as_ref() else { return };
    let Ok(conn) = lock.lock() else { return };
    let cp = CachedParse::from_parsed(pf, arena);
    if let Ok(payload) = serde_json::to_string(&cp) {
        let _ = conn.execute(
            "INSERT OR REPLACE INTO parse_cache (key, payload) VALUES (?1, ?2)",
            rusqlite::params![cache_key(abs_path, content_hash), payload],
        );
    }
}
