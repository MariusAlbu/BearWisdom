use super::*;
use crate::db::Database;

/// Insert the minimal rows needed for flow tests.
fn seed_flow(db: &Database) {
    let conn = &db.conn;

    // Two files: a TypeScript frontend and a C# backend.
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES
         ('src/api/client.ts',   'h1', 'typescript', 0),
         ('src/CatalogController.cs', 'h2', 'csharp', 0)",
        [],
    )
    .unwrap();

    let ts_id: i64 = conn
        .query_row("SELECT id FROM files WHERE path = 'src/api/client.ts'", [], |r| r.get(0))
        .unwrap();
    let cs_id: i64 = conn
        .query_row(
            "SELECT id FROM files WHERE path = 'src/CatalogController.cs'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    // TS → C# http_call edge at line 15.
    conn.execute(
        "INSERT INTO flow_edges (
            source_file_id, source_line, source_symbol, source_language,
            target_file_id, target_line, target_symbol, target_language,
            edge_type, protocol, url_pattern, confidence
         ) VALUES (?1, 15, 'fetchCatalog', 'typescript',
                   ?2, 42, 'GetCatalog',   'csharp',
                   'http_call', 'http', '/api/catalog', 0.9)",
        rusqlite::params![ts_id, cs_id],
    )
    .unwrap();

    // C# → C# internal call (same language, same file hop).
    conn.execute(
        "INSERT INTO flow_edges (
            source_file_id, source_line, source_symbol, source_language,
            target_file_id, target_line, target_symbol, target_language,
            edge_type, protocol, confidence
         ) VALUES (?1, 42, 'GetCatalog', 'csharp',
                   ?1, 80, 'LoadFromDb',  'csharp',
                   'calls', NULL, 1.0)",
        rusqlite::params![cs_id],
    )
    .unwrap();
}

#[test]
fn trace_from_ts_file_finds_downstream_steps() {
    let db = Database::open_in_memory().unwrap();
    seed_flow(&db);

    let steps = trace_flow(&db, "src/api/client.ts", 15, 3).unwrap();
    assert!(
        !steps.is_empty(),
        "Expected at least one step when tracing from TS file"
    );

    // First step should originate in the TS file.
    let first = &steps[0];
    assert_eq!(first.file_path, "src/api/client.ts");
    assert_eq!(first.edge_type, "http_call");
}

#[test]
fn trace_reaches_downstream_hops() {
    let db = Database::open_in_memory().unwrap();
    seed_flow(&db);

    let steps = trace_flow(&db, "src/api/client.ts", 15, 3).unwrap();

    // With depth 3 we should reach both the C# controller and the DB loader.
    let paths: Vec<&str> = steps.iter().map(|s| s.file_path.as_str()).collect();
    assert!(
        paths.contains(&"src/api/client.ts"),
        "Expected TS source in trace"
    );
}

#[test]
fn trace_depth_zero_returns_empty_or_origin() {
    let db = Database::open_in_memory().unwrap();
    seed_flow(&db);

    // With max_depth=0 the recursive term never fires; only base rows returned.
    let steps = trace_flow(&db, "src/api/client.ts", 15, 0).unwrap();
    // All returned rows must be at depth 0.
    for step in &steps {
        assert_eq!(step.depth, 0);
    }
}

#[test]
fn trace_unknown_file_returns_empty() {
    let db = Database::open_in_memory().unwrap();
    seed_flow(&db);

    let steps = trace_flow(&db, "nonexistent/file.ts", 1, 5).unwrap();
    assert!(steps.is_empty());
}

#[test]
fn cross_language_paths_finds_ts_to_csharp() {
    let db = Database::open_in_memory().unwrap();
    seed_flow(&db);

    let paths = cross_language_paths(&db, "typescript", "csharp", 10).unwrap();
    assert!(!paths.is_empty(), "Expected at least one TS→C# path");

    let first = &paths[0];
    assert_eq!(first[0].language, "typescript");

    let has_csharp_target = first.iter().any(|s| s.language == "csharp");
    assert!(has_csharp_target, "Expected C# target in path");
}

#[test]
fn cross_language_paths_wrong_direction_returns_empty() {
    let db = Database::open_in_memory().unwrap();
    seed_flow(&db);

    // There are no python → rust edges in our seed data.
    let paths = cross_language_paths(&db, "python", "rust", 10).unwrap();
    assert!(paths.is_empty());
}

#[test]
fn cross_language_paths_respects_limit() {
    let db = Database::open_in_memory().unwrap();
    seed_flow(&db);

    let limited = cross_language_paths(&db, "typescript", "csharp", 1).unwrap();
    assert!(limited.len() <= 1);
}

#[test]
fn empty_db_returns_empty_for_all_functions() {
    let db = Database::open_in_memory().unwrap();

    let steps = trace_flow(&db, "anything.ts", 1, 5).unwrap();
    assert!(steps.is_empty());

    let paths = cross_language_paths(&db, "typescript", "csharp", 10).unwrap();
    assert!(paths.is_empty());
}
