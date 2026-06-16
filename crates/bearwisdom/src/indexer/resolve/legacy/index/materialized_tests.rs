use super::*;
use std::sync::Arc;

fn sym(id: i64, name: &str, qname: &str, kind: &str) -> SymbolInfo {
    SymbolInfo {
        id,
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: kind.to_string(),
        visibility: None,
        file_path: Arc::from("ext:pkg/lib.rs"),
        scope_path: None,
        package_id: None,
        signature: None,
    }
}

#[test]
fn intern_and_lookup_by_name_qname_parent() {
    let store = MaterializedStore::new();
    store.intern(sym(1, "Open", "db.Open", "function"));
    store.intern(sym(2, "Query", "db.Conn.Query", "method"));

    assert_eq!(store.by_name("Open").len(), 1);
    assert_eq!(store.by_name("Open")[0].qualified_name, "db.Open");
    assert_eq!(store.by_qualified_name("db.Conn.Query").map(|s| s.id), Some(2));
    let members = store.members_of("db.Conn");
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].name, "Query");
}

#[test]
fn miss_returns_empty() {
    let store = MaterializedStore::new();
    assert!(store.is_empty());
    assert!(store.by_name("Nope").is_empty());
    assert!(store.by_qualified_name("a.b").is_none());
    assert!(store.members_of("a").is_empty());
}

#[test]
fn first_writer_wins_on_qname() {
    let store = MaterializedStore::new();
    store.intern(sym(1, "Foo", "m.Foo", "class"));
    store.intern(sym(2, "Foo", "m.Foo", "interface"));
    // by_qname keeps the first; by_name keeps both (declaration merging).
    assert_eq!(store.by_qualified_name("m.Foo").map(|s| s.id), Some(1));
    assert_eq!(store.by_name("Foo").len(), 2);
}

#[test]
fn borrows_stay_valid_across_later_interns() {
    // The load-bearing property: a reference handed out before further interns
    // must remain valid after the store grows. `boxcar::Vec` gives stable
    // element addresses, so the early borrow is not invalidated.
    let store = MaterializedStore::new();
    store.intern(sym(1, "First", "m.First", "class"));
    let early: &SymbolInfo = store.by_name("First").into_iter().next().unwrap();
    // Force many subsequent pushes — a plain Vec would reallocate here.
    for i in 0..1000 {
        store.intern(sym(100 + i, "Filler", &format!("m.Filler{i}"), "function"));
    }
    // The early borrow is still readable and correct.
    assert_eq!(early.qualified_name, "m.First");
    assert_eq!(store.len(), 1001);
}

#[test]
fn intern_under_shared_ref_across_threads() {
    // `intern` takes `&self`, mirroring the resolve pass where the store is
    // shared by `&` across rayon workers. Concurrent interns must all land.
    let store = MaterializedStore::new();
    std::thread::scope(|scope| {
        for t in 0..8 {
            let s = &store;
            scope.spawn(move || {
                for i in 0..50 {
                    let n = t * 50 + i;
                    s.intern(sym(n as i64, "X", &format!("m.X{n}"), "function"));
                }
            });
        }
    });
    assert_eq!(store.len(), 400);
    assert_eq!(store.by_name("X").len(), 400);
}

#[test]
fn file_guard_runs_init_once() {
    let store = MaterializedStore::new();
    let path = std::path::Path::new("ext:pkg/a.rs");
    let counter = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let s = &store;
            let c = &counter;
            scope.spawn(move || {
                let guard = s.file_guard(path);
                guard.get_or_init(|| {
                    c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                });
            });
        }
    });
    // Exactly one worker ran the init closure despite 8 racing for the file.
    assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
}
