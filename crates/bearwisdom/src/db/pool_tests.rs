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
        let _: i64 = db.query_row("SELECT 1", [], |r| r.get(0)).unwrap();
    }

    // Return all four — only 2 should be kept (max_size).
    drop(db1);
    drop(db2);
    drop(db3);
    drop(db4);

    // Verify pool still works after returns.
    let db = pool.get().unwrap();
    let _: i64 = db.query_row("SELECT 1", [], |r| r.get(0)).unwrap();
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
