use super::*;
use crate::db::Database;

fn open() -> Database {
    Database::open_in_memory().unwrap()
}

fn insert_file(db: &Database, path: &str) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed) \
             VALUES (?1, 'h', 'rust', 0)",
            [path],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

fn insert_symbol(db: &Database, file_id: i64, name: &str) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col) \
             VALUES (?1, ?2, ?2, 'function', 1, 0)",
            rusqlite::params![file_id, name],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

fn insert_edge(db: &Database, src: i64, tgt: i64, conf: f64, kind: &str) {
    db.conn()
        .execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence) \
             VALUES (?1, ?2, ?3, 1, ?4)",
            rusqlite::params![src, tgt, kind, conf],
        )
        .unwrap();
}

fn insert_entry_point(db: &Database, sym: i64) {
    db.conn()
        .execute(
            "INSERT INTO entry_points (symbol_id, kind, source) \
             VALUES (?1, 'main', 'test')",
            [sym],
        )
        .unwrap();
}

fn reach_row(db: &Database, sym: i64) -> Option<(u32, f64, Option<String>)> {
    db.conn()
        .query_row(
            "SELECT min_distance, path_confidence, via_kind \
             FROM reachability WHERE symbol_id = ?1",
            [sym],
            |r| Ok((r.get::<_, u32>(0)?, r.get::<_, f64>(1)?, r.get::<_, Option<String>>(2)?)),
        )
        .ok()
}

#[test]
fn empty_db_yields_empty_reachability() {
    let db = open();
    materialize_reachability(&db).unwrap();
    let count: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM reachability", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn entry_point_is_reachable_at_distance_zero() {
    let db = open();
    let f = insert_file(&db, "src/main.rs");
    let main = insert_symbol(&db, f, "main");
    insert_entry_point(&db, main);

    materialize_reachability(&db).unwrap();
    let r = reach_row(&db, main).expect("entry point must appear in reachability");
    assert_eq!(r.0, 0);
    assert!((r.1 - 1.0).abs() < 1e-9, "entry-point conf must be 1.0, got {}", r.1);
    assert!(r.2.is_none(), "entry-point via_kind must be NULL");
}

#[test]
fn linear_chain_assigns_increasing_distance() {
    let db = open();
    let f = insert_file(&db, "src/lib.rs");
    let a = insert_symbol(&db, f, "ep");
    let b = insert_symbol(&db, f, "middle");
    let c = insert_symbol(&db, f, "leaf");
    insert_entry_point(&db, a);
    insert_edge(&db, a, b, 1.0, "calls");
    insert_edge(&db, b, c, 1.0, "calls");

    materialize_reachability(&db).unwrap();
    assert_eq!(reach_row(&db, a).unwrap().0, 0);
    assert_eq!(reach_row(&db, b).unwrap().0, 1);
    assert_eq!(reach_row(&db, c).unwrap().0, 2);
}

#[test]
fn edges_below_confidence_threshold_are_ignored() {
    let db = open();
    let f = insert_file(&db, "src/lib.rs");
    let a = insert_symbol(&db, f, "ep");
    let b = insert_symbol(&db, f, "weakly_called");
    insert_entry_point(&db, a);
    insert_edge(&db, a, b, 0.3, "heuristic");

    materialize_reachability(&db).unwrap();
    assert_eq!(reach_row(&db, a).unwrap().0, 0);
    assert!(
        reach_row(&db, b).is_none(),
        "edge below threshold must NOT carry reachability"
    );
}

#[test]
fn cycle_terminates_and_assigns_first_visit_distance() {
    let db = open();
    let f = insert_file(&db, "src/lib.rs");
    let a = insert_symbol(&db, f, "ep");
    let b = insert_symbol(&db, f, "b");
    let c = insert_symbol(&db, f, "c");
    insert_entry_point(&db, a);
    insert_edge(&db, a, b, 1.0, "calls");
    insert_edge(&db, b, c, 1.0, "calls");
    insert_edge(&db, c, b, 1.0, "calls"); // back-edge forming a cycle

    materialize_reachability(&db).unwrap();
    assert_eq!(reach_row(&db, b).unwrap().0, 1);
    assert_eq!(reach_row(&db, c).unwrap().0, 2);
}

#[test]
fn path_confidence_takes_min_along_path() {
    let db = open();
    let f = insert_file(&db, "src/lib.rs");
    let a = insert_symbol(&db, f, "ep");
    let b = insert_symbol(&db, f, "b");
    let c = insert_symbol(&db, f, "c");
    insert_entry_point(&db, a);
    insert_edge(&db, a, b, 0.9, "calls");
    insert_edge(&db, b, c, 0.6, "dispatch");

    materialize_reachability(&db).unwrap();
    let rc = reach_row(&db, c).unwrap();
    assert!((rc.1 - 0.6).abs() < 1e-9, "c's path_confidence must be min(0.9, 0.6) = 0.6, got {}", rc.1);
    assert_eq!(rc.2.as_deref(), Some("dispatch"));
}

#[test]
fn convergent_paths_prefer_higher_confidence_at_same_distance() {
    let db = open();
    let f = insert_file(&db, "src/lib.rs");
    let ep1 = insert_symbol(&db, f, "ep1");
    let ep2 = insert_symbol(&db, f, "ep2");
    let target = insert_symbol(&db, f, "target");
    insert_entry_point(&db, ep1);
    insert_entry_point(&db, ep2);
    insert_edge(&db, ep1, target, 0.5, "heuristic"); // distance 1, conf 0.5
    insert_edge(&db, ep2, target, 1.0, "calls");      // distance 1, conf 1.0

    materialize_reachability(&db).unwrap();
    let r = reach_row(&db, target).unwrap();
    assert_eq!(r.0, 1);
    assert!((r.1 - 1.0).abs() < 1e-9, "tied-distance paths must keep highest conf, got {}", r.1);
    assert_eq!(r.2.as_deref(), Some("calls"));
}

#[test]
fn idempotent_rebuild_clears_stale_rows() {
    let db = open();
    let f = insert_file(&db, "src/lib.rs");
    let a = insert_symbol(&db, f, "ep");
    let b = insert_symbol(&db, f, "b");
    insert_entry_point(&db, a);
    insert_edge(&db, a, b, 1.0, "calls");

    materialize_reachability(&db).unwrap();
    let first: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM reachability", [], |r| r.get(0))
        .unwrap();
    assert_eq!(first, 2);

    // Drop the edge — rebuild must remove b from reachability.
    db.conn().execute("DELETE FROM edges", []).unwrap();
    materialize_reachability(&db).unwrap();
    let second: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM reachability", [], |r| r.get(0))
        .unwrap();
    assert_eq!(second, 1, "after dropping edges, only the entry point should remain");
}

#[test]
fn unreachable_symbols_are_omitted() {
    let db = open();
    let f = insert_file(&db, "src/lib.rs");
    let main = insert_symbol(&db, f, "main");
    let dead = insert_symbol(&db, f, "dead");
    insert_entry_point(&db, main);
    // No edge from main to dead.

    materialize_reachability(&db).unwrap();
    assert!(reach_row(&db, main).is_some());
    assert!(
        reach_row(&db, dead).is_none(),
        "symbol unreachable from any entry point must be absent from the table"
    );
}
