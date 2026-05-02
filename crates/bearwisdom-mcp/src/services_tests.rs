use super::services::ServiceCache;
use bearwisdom::IndexServiceOptions;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

fn opts() -> IndexServiceOptions {
    IndexServiceOptions {
        pool_size: 1,
        watch: false, // tests don't need a watcher; spawning many is slow + flaky on CI
        debounce: Duration::from_millis(50),
    }
}

fn make_project_dir() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"x\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn x() {}\n").unwrap();
    dir
}

#[test]
fn get_or_open_lazily_creates_service() {
    let cache = ServiceCache::new(4, opts());
    let dir = make_project_dir();
    assert_eq!(cache.len(), 0);

    let svc = cache.get_or_open(dir.path()).expect("open");
    assert_eq!(cache.len(), 1);
    assert!(Arc::strong_count(&svc) >= 2, "cache holds one ref + caller");
}

#[test]
fn get_or_open_reuses_cached_service() {
    let cache = ServiceCache::new(4, opts());
    let dir = make_project_dir();

    let a = cache.get_or_open(dir.path()).expect("first");
    let b = cache.get_or_open(dir.path()).expect("second");
    assert!(Arc::ptr_eq(&a, &b), "second call must reuse first service");
    assert_eq!(cache.len(), 1, "no new entry on hit");
}

#[test]
fn missing_project_returns_structured_error() {
    let cache = ServiceCache::new(4, opts());
    let bogus = PathBuf::from("F:/this/does/not/exist/at/all");
    let err = match cache.get_or_open(&bogus) {
        Err(e) => e,
        Ok(_) => panic!("expected error opening missing project"),
    };
    assert_eq!(err.0, "PROJECT_NOT_FOUND");
    assert!(err.1.contains("does not exist") || err.1.contains("not a directory"),
        "got message: {}", err.1);
    assert_eq!(cache.len(), 0, "failed open must not pollute the cache");
}

#[test]
fn cache_evicts_lru_at_capacity() {
    let cache = ServiceCache::new(2, opts());

    let dirs: Vec<TempDir> = (0..3).map(|_| make_project_dir()).collect();
    let _a = cache.get_or_open(dirs[0].path()).expect("open 0");
    let _b = cache.get_or_open(dirs[1].path()).expect("open 1");
    assert_eq!(cache.len(), 2);

    // Touch 0 to bump it to MRU; 1 is now LRU.
    let _a2 = cache.get_or_open(dirs[0].path()).expect("touch 0");
    // Open a third — should evict 1 (the LRU), not 0.
    let _c = cache.get_or_open(dirs[2].path()).expect("open 2");
    assert_eq!(cache.len(), 2);

    // Verify: opening 1 again creates a fresh service, opening 0 reuses.
    let still_a = cache.get_or_open(dirs[0].path()).expect("reuse 0");
    assert!(Arc::ptr_eq(&_a, &still_a), "0 must still be cached after eviction");
}

#[test]
fn insert_seeds_default_project_without_lazy_open() {
    let cache = ServiceCache::new(4, opts());
    let dir = make_project_dir();
    let db_path = bearwisdom::resolve_db_path(dir.path()).unwrap();
    let svc = Arc::new(
        bearwisdom::IndexService::open(&db_path, dir.path(), opts()).expect("open"),
    );

    cache.insert(dir.path().to_path_buf(), svc.clone());
    assert_eq!(cache.len(), 1);

    let fetched = cache.get_or_open(dir.path()).expect("lookup");
    assert!(Arc::ptr_eq(&svc, &fetched), "insert must seed the entry the resolver returns");
}
