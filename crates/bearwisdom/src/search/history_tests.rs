use super::*;
use crate::db::Database;

#[test]
fn record_and_retrieve() {
    let db = Database::open_in_memory().unwrap();
    record_search(&db.conn, "CatalogItem", "symbol", None).unwrap();

    let results = recent_searches(&db.conn, None, 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].query, "CatalogItem");
    assert_eq!(results[0].use_count, 1);
}

#[test]
fn repeated_search_increments_count() {
    let db = Database::open_in_memory().unwrap();
    record_search(&db.conn, "test", "grep", None).unwrap();
    record_search(&db.conn, "test", "grep", None).unwrap();
    record_search(&db.conn, "test", "grep", None).unwrap();

    let results = recent_searches(&db.conn, Some("grep"), 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].use_count, 3);
}

#[test]
fn save_and_unsave() {
    let db = Database::open_in_memory().unwrap();
    record_search(&db.conn, "important", "symbol", None).unwrap();

    let results = recent_searches(&db.conn, None, 10).unwrap();
    let id = results[0].id;
    assert!(!results[0].is_saved);

    let now_saved = toggle_saved(&db.conn, id).unwrap();
    assert!(now_saved);

    let saved = saved_searches(&db.conn).unwrap();
    assert_eq!(saved.len(), 1);

    let now_unsaved = toggle_saved(&db.conn, id).unwrap();
    assert!(!now_unsaved);
}

#[test]
fn prune_keeps_recent_and_saved() {
    let db = Database::open_in_memory().unwrap();
    for i in 0..10 {
        record_search(&db.conn, &format!("query{i}"), "grep", None).unwrap();
    }
    // Save one entry so it survives pruning
    let all = recent_searches(&db.conn, None, 100).unwrap();
    toggle_saved(&db.conn, all[0].id).unwrap();

    let deleted = prune_history(&db.conn, 3).unwrap();
    assert_eq!(deleted, 6); // 10 total, 1 saved, keep 3 unsaved = delete 6

    let remaining = recent_searches(&db.conn, None, 100).unwrap();
    assert_eq!(remaining.len(), 4); // 3 unsaved + 1 saved
}

#[test]
fn filter_by_query_type() {
    let db = Database::open_in_memory().unwrap();
    record_search(&db.conn, "foo", "grep", None).unwrap();
    record_search(&db.conn, "bar", "symbol", None).unwrap();
    record_search(&db.conn, "baz", "grep", None).unwrap();

    let grep_only = recent_searches(&db.conn, Some("grep"), 10).unwrap();
    assert_eq!(grep_only.len(), 2);
    assert!(grep_only.iter().all(|e| e.query_type == "grep"));
}
