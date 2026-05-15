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

fn insert_type(
    db: &Database,
    file_id: i64,
    name: &str,
    qname: &str,
    kind: &str,
) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO symbols \
                (file_id, name, qualified_name, kind, line, col, origin) \
             VALUES (?1, ?2, ?3, ?4, 1, 0, 'internal')",
            rusqlite::params![file_id, name, qname, kind],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

fn insert_method(
    db: &Database,
    file_id: i64,
    name: &str,
    parent_qname: &str,
) -> i64 {
    let qname = format!("{parent_qname}::{name}");
    db.conn()
        .execute(
            "INSERT INTO symbols \
                (file_id, name, qualified_name, kind, line, col, scope_path, origin) \
             VALUES (?1, ?2, ?3, 'method', 1, 0, ?4, 'internal')",
            rusqlite::params![file_id, name, qname, parent_qname],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

fn insert_inherits(db: &Database, child: i64, parent: i64) {
    db.conn()
        .execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence) \
             VALUES (?1, ?2, 'inherits', 1, 1.0)",
            rusqlite::params![child, parent],
        )
        .unwrap();
}

fn insert_implements(db: &Database, child: i64, parent: i64) {
    db.conn()
        .execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence) \
             VALUES (?1, ?2, 'implements', 1, 1.0)",
            rusqlite::params![child, parent],
        )
        .unwrap();
}

fn dispatch_targets(db: &Database, source_id: i64) -> Vec<i64> {
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT target_id FROM edges \
             WHERE kind = 'dispatch_candidate' AND source_id = ?1 \
             ORDER BY target_id",
        )
        .unwrap();
    stmt.query_map([source_id], |r| r.get::<_, i64>(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
}

fn count_dispatch_edges(db: &Database) -> i64 {
    db.conn()
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE kind = 'dispatch_candidate'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn empty_db_emits_no_edges() {
    let db = open();
    let n = synthesize_dispatch_edges(&db).unwrap();
    assert_eq!(n, 0);
}

#[test]
fn interface_method_dispatches_to_single_impl() {
    let db = open();
    let f = insert_file(&db, "src/lib.rs");
    let iface = insert_type(&db, f, "Animal", "Animal", "interface");
    let cls = insert_type(&db, f, "Dog", "Dog", "class");
    let iface_speak = insert_method(&db, f, "speak", "Animal");
    let impl_speak = insert_method(&db, f, "speak", "Dog");
    insert_implements(&db, cls, iface);

    synthesize_dispatch_edges(&db).unwrap();
    let tgts = dispatch_targets(&db, iface_speak);
    assert_eq!(tgts, vec![impl_speak]);
}

#[test]
fn fanout_across_all_impls() {
    let db = open();
    let f = insert_file(&db, "src/lib.rs");
    let iface = insert_type(&db, f, "Animal", "Animal", "interface");
    let dog = insert_type(&db, f, "Dog", "Dog", "class");
    let cat = insert_type(&db, f, "Cat", "Cat", "class");
    let iface_speak = insert_method(&db, f, "speak", "Animal");
    let dog_speak = insert_method(&db, f, "speak", "Dog");
    let cat_speak = insert_method(&db, f, "speak", "Cat");
    insert_implements(&db, dog, iface);
    insert_implements(&db, cat, iface);

    synthesize_dispatch_edges(&db).unwrap();
    let mut tgts = dispatch_targets(&db, iface_speak);
    tgts.sort();
    let mut want = vec![dog_speak, cat_speak];
    want.sort();
    assert_eq!(tgts, want);
}

#[test]
fn transitive_inheritance_reaches_grandchild() {
    let db = open();
    let f = insert_file(&db, "src/lib.rs");
    let base = insert_type(&db, f, "Base", "Base", "abstract_class");
    let mid = insert_type(&db, f, "Mid", "Mid", "class");
    let leaf = insert_type(&db, f, "Leaf", "Leaf", "class");
    let base_run = insert_method(&db, f, "run", "Base");
    let _mid_run = insert_method(&db, f, "run", "Mid");
    let leaf_run = insert_method(&db, f, "run", "Leaf");
    insert_inherits(&db, mid, base);
    insert_inherits(&db, leaf, mid);

    synthesize_dispatch_edges(&db).unwrap();
    let tgts = dispatch_targets(&db, base_run);
    // Both Mid::run and Leaf::run are valid dispatch targets from Base::run.
    assert!(tgts.contains(&leaf_run), "transitive grandchild must be a dispatch target");
    assert_eq!(tgts.len(), 2, "expected dispatch to both Mid::run and Leaf::run");
}

#[test]
fn no_edge_when_subclass_doesnt_override() {
    let db = open();
    let f = insert_file(&db, "src/lib.rs");
    let iface = insert_type(&db, f, "Animal", "Animal", "interface");
    let cls = insert_type(&db, f, "Dog", "Dog", "class");
    let iface_speak = insert_method(&db, f, "speak", "Animal");
    // Dog has a different method, not `speak`.
    let _bark = insert_method(&db, f, "bark", "Dog");
    insert_implements(&db, cls, iface);

    synthesize_dispatch_edges(&db).unwrap();
    assert!(
        dispatch_targets(&db, iface_speak).is_empty(),
        "no override means no dispatch edge"
    );
}

#[test]
fn idempotent_rerun_keeps_count_stable() {
    let db = open();
    let f = insert_file(&db, "src/lib.rs");
    let iface = insert_type(&db, f, "I", "I", "interface");
    let cls = insert_type(&db, f, "C", "C", "class");
    insert_method(&db, f, "m", "I");
    insert_method(&db, f, "m", "C");
    insert_implements(&db, cls, iface);

    synthesize_dispatch_edges(&db).unwrap();
    let first = count_dispatch_edges(&db);
    synthesize_dispatch_edges(&db).unwrap();
    let second = count_dispatch_edges(&db);
    assert_eq!(first, second);
    assert_eq!(first, 1);
}

#[test]
fn synthesized_edges_use_dispatch_candidate_confidence() {
    let db = open();
    let f = insert_file(&db, "src/lib.rs");
    let iface = insert_type(&db, f, "I", "I", "interface");
    let cls = insert_type(&db, f, "C", "C", "class");
    insert_method(&db, f, "m", "I");
    insert_method(&db, f, "m", "C");
    insert_implements(&db, cls, iface);

    synthesize_dispatch_edges(&db).unwrap();
    let conf: f64 = db
        .conn()
        .query_row(
            "SELECT confidence FROM edges WHERE kind = 'dispatch_candidate'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!((conf - DISPATCH_CANDIDATE_CONFIDENCE).abs() < 1e-9);
}

#[test]
fn external_parent_still_dispatches_to_internal_override() {
    // Internal class extending a node_modules base class should still
    // get a dispatch edge so internal overrides aren't flagged dead.
    let db = open();
    let f = insert_file(&db, "src/lib.rs");
    db.conn()
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed, origin) \
             VALUES ('ext:react/index.d.ts', 'h', 'typescript', 0, 'external')",
            [],
        )
        .unwrap();
    let ext_file = db.conn().last_insert_rowid();

    // External base type + method.
    db.conn()
        .execute(
            "INSERT INTO symbols \
                (file_id, name, qualified_name, kind, line, col, origin) \
             VALUES (?1, 'Component', 'Component', 'class', 1, 0, 'external')",
            [ext_file],
        )
        .unwrap();
    let ext_class = db.conn().last_insert_rowid();
    db.conn()
        .execute(
            "INSERT INTO symbols \
                (file_id, name, qualified_name, kind, line, col, scope_path, origin) \
             VALUES (?1, 'render', 'Component::render', 'method', 1, 0, 'Component', 'external')",
            [ext_file],
        )
        .unwrap();
    let ext_render = db.conn().last_insert_rowid();

    // Internal subclass + override.
    let cls = insert_type(&db, f, "MyComponent", "MyComponent", "class");
    let int_render = insert_method(&db, f, "render", "MyComponent");
    insert_inherits(&db, cls, ext_class);

    synthesize_dispatch_edges(&db).unwrap();
    let tgts = dispatch_targets(&db, ext_render);
    assert_eq!(tgts, vec![int_render]);
}

#[test]
fn rerun_clears_stale_edges_when_subclass_deleted() {
    let db = open();
    let f = insert_file(&db, "src/lib.rs");
    let iface = insert_type(&db, f, "I", "I", "interface");
    let cls = insert_type(&db, f, "C", "C", "class");
    insert_method(&db, f, "m", "I");
    insert_method(&db, f, "m", "C");
    insert_implements(&db, cls, iface);

    synthesize_dispatch_edges(&db).unwrap();
    assert_eq!(count_dispatch_edges(&db), 1);

    // Drop the inheritance edge — re-run must clear the dispatch row.
    db.conn().execute("DELETE FROM edges WHERE kind = 'implements'", []).unwrap();
    synthesize_dispatch_edges(&db).unwrap();
    assert_eq!(count_dispatch_edges(&db), 0);
}
