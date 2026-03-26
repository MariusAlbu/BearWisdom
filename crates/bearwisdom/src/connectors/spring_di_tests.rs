use super::*;
use crate::db::Database;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Seed a minimal graph: one @Service class implementing one interface.
///
/// Returns (iface_symbol_id, impl_symbol_id).
fn seed_service_implements_interface(db: &Database) -> (i64, i64) {
    let conn = &db.conn;

    // Two files — one for the interface, one for the implementation.
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('src/IOrderService.java', 'h1', 'java', 0)",
        [],
    )
    .unwrap();
    let iface_file_id: i64 = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('src/OrderService.java', 'h2', 'java', 0)",
        [],
    )
    .unwrap();
    let impl_file_id: i64 = conn.last_insert_rowid();

    // Interface symbol.
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'IOrderService', 'com.example.IOrderService', 'interface', 3, 0)",
        [iface_file_id],
    )
    .unwrap();
    let iface_symbol_id: i64 = conn.last_insert_rowid();

    // Implementation class symbol.
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'OrderService', 'com.example.OrderService', 'class', 5, 0)",
        [impl_file_id],
    )
    .unwrap();
    let impl_symbol_id: i64 = conn.last_insert_rowid();

    // implements edge: OrderService → IOrderService.
    conn.execute(
        "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
         VALUES (?1, ?2, 'implements', 5, 0.90)",
        [impl_symbol_id, iface_symbol_id],
    )
    .unwrap();

    (iface_symbol_id, impl_symbol_id)
}

/// Register `impl_symbol_id` as a member of the "spring-services" concept.
fn register_spring_service_concept(db: &Database, impl_symbol_id: i64) {
    let conn = &db.conn;

    conn.execute(
        "INSERT OR IGNORE INTO concepts (name, description)
         VALUES ('spring-services', 'Spring @Service classes')",
        [],
    )
    .unwrap();

    let concept_id: i64 = conn
        .query_row(
            "SELECT id FROM concepts WHERE name = 'spring-services'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    conn.execute(
        "INSERT OR IGNORE INTO concept_members (concept_id, symbol_id, auto_assigned)
         VALUES (?1, ?2, 1)",
        rusqlite::params![concept_id, impl_symbol_id],
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A @Service class with an `implements` edge produces one di_binding flow_edge.
#[test]
fn service_with_implements_edge_creates_di_binding() {
    let db = Database::open_in_memory().unwrap();
    let (_iface_id, impl_id) = seed_service_implements_interface(&db);
    register_spring_service_concept(&db, impl_id);

    let created = connect(&db.conn, std::path::Path::new(".")).unwrap();
    assert_eq!(created, 1, "Expected one di_binding flow_edge");

    let count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM flow_edges WHERE edge_type = 'di_binding'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

/// Verify the flow_edge fields are populated correctly.
#[test]
fn di_binding_flow_edge_fields_are_correct() {
    let db = Database::open_in_memory().unwrap();
    let (_iface_id, impl_id) = seed_service_implements_interface(&db);
    register_spring_service_concept(&db, impl_id);

    connect(&db.conn, std::path::Path::new(".")).unwrap();

    let (source_symbol, target_symbol, edge_type, source_language, target_language): (
        String,
        String,
        String,
        String,
        String,
    ) = db
        .conn
        .query_row(
            "SELECT source_symbol, target_symbol, edge_type, source_language, target_language
             FROM flow_edges WHERE edge_type = 'di_binding' LIMIT 1",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                ))
            },
        )
        .unwrap();

    assert_eq!(source_symbol, "OrderService");
    assert_eq!(target_symbol, "IOrderService");
    assert_eq!(edge_type, "di_binding");
    assert_eq!(source_language, "java");
    assert_eq!(target_language, "java");
}

/// No stereotype concept members and no Java files → 0 flow_edges, no error.
#[test]
fn no_services_produces_zero_edges() {
    let db = Database::open_in_memory().unwrap();

    let created = connect(&db.conn, std::path::Path::new(".")).unwrap();
    assert_eq!(created, 0, "Expected zero flow_edges when no services exist");

    let count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM flow_edges",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

/// A @Service class with no `implements` edge produces no di_binding.
/// (Class is injectable but doesn't satisfy a known interface.)
#[test]
fn service_without_implements_edge_produces_no_binding() {
    let db = Database::open_in_memory().unwrap();

    // Insert a file and a class symbol without any implements edge.
    db.conn
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/StandaloneService.java', 'h1', 'java', 0)",
            [],
        )
        .unwrap();
    let file_id: i64 = db.conn.last_insert_rowid();

    db.conn
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
             VALUES (?1, 'StandaloneService', 'com.example.StandaloneService', 'class', 3, 0)",
            [file_id],
        )
        .unwrap();
    let impl_id: i64 = db.conn.last_insert_rowid();

    register_spring_service_concept(&db, impl_id);

    let created = connect(&db.conn, std::path::Path::new(".")).unwrap();
    assert_eq!(created, 0, "No interface binding without implements edge");
}

/// Running connect twice does not double-insert (OR IGNORE semantics).
#[test]
fn idempotent_on_repeated_run() {
    let db = Database::open_in_memory().unwrap();
    let (_iface_id, impl_id) = seed_service_implements_interface(&db);
    register_spring_service_concept(&db, impl_id);

    connect(&db.conn, std::path::Path::new(".")).unwrap();
    let second_run = connect(&db.conn, std::path::Path::new(".")).unwrap();
    assert_eq!(second_run, 0, "Second run must not insert duplicates");

    let count: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM flow_edges WHERE edge_type = 'di_binding'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "Exactly one binding after two runs");
}
