use super::service::{IndexService, IndexServiceOptions, ReindexStats};
use std::time::Duration;
use tempfile::TempDir;

fn write_rust_file(dir: &std::path::Path, name: &str, contents: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn make_minimal_project() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_rust_file(
        root,
        "Cargo.toml",
        "[package]\nname = \"svc-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write_rust_file(root, "src/lib.rs", "pub fn hello() -> u32 { 42 }\n");
    dir
}

fn open_no_watch(root: &std::path::Path) -> IndexService {
    let db_path = bearwisdom::resolve_db_path(root).unwrap();
    let opts = IndexServiceOptions {
        pool_size: 1,
        watch: false,
        debounce: Duration::from_millis(100),
    };
    IndexService::open(&db_path, root, opts).unwrap()
}

// `bearwisdom::resolve_db_path` lives at the crate root; bring it in.
use crate as bearwisdom;

#[test]
fn open_without_watch_provides_working_pool() {
    let dir = make_minimal_project();
    let service = open_no_watch(dir.path());

    // Sanity: the pool is alive and produces a connection.
    let _db = service.pool().get().expect("pool acquire");
    assert_eq!(service.project_root(), dir.path());
}

#[test]
fn reindex_now_full_then_incremental() {
    let dir = make_minimal_project();
    let service = open_no_watch(dir.path());

    // First pass: empty DB → Full.
    let first = service.reindex_now().expect("first reindex");
    assert!(matches!(first, ReindexStats::Full(_)), "expected Full, got {first:?}");
    if let ReindexStats::Full(stats) = first {
        assert!(stats.file_count > 0, "no files indexed");
    }

    // Second pass: DB now populated → Incremental.
    let second = service.reindex_now().expect("second reindex");
    assert!(
        matches!(second, ReindexStats::Incremental(_)),
        "expected Incremental on second pass, got {second:?}",
    );
}

#[test]
fn open_does_not_run_initial_reindex_implicitly() {
    let dir = make_minimal_project();
    let service = open_no_watch(dir.path());

    // No `reindex_now` called yet → DB should have zero indexed files.
    let db = service.pool().get().unwrap();
    let file_count: i64 = db
        .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
        .unwrap_or(0);
    assert_eq!(
        file_count, 0,
        "open() must not run an implicit reindex; caller is responsible for calling reindex_now()",
    );
}

#[test]
fn reindex_now_writes_last_indexed_at_meta() {
    let dir = make_minimal_project();
    let service = open_no_watch(dir.path());

    // Before any reindex, the meta key should be absent.
    {
        let db = service.pool().get().unwrap();
        assert!(bearwisdom::last_indexed_at_ms(&db).is_none());
    }

    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    service.reindex_now().expect("reindex");
    let after = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let db = service.pool().get().unwrap();
    let ts = bearwisdom::last_indexed_at_ms(&db).expect("meta key written");
    assert!(
        ts >= before && ts <= after,
        "expected timestamp between {before} and {after}, got {ts}",
    );
}

#[test]
fn watcher_thread_starts_and_stops_cleanly() {
    let dir = make_minimal_project();
    let db_path = bearwisdom::resolve_db_path(dir.path()).unwrap();
    let opts = IndexServiceOptions {
        pool_size: 1,
        watch: true,
        debounce: Duration::from_millis(100),
    };
    let service = IndexService::open(&db_path, dir.path(), opts).expect("open with watch");
    // Drop the service; the watcher thread should exit on its own (channel
    // disconnect from the dropped `notify::Watcher`). Test passes if drop
    // returns within a reasonable window.
    drop(service);
}
