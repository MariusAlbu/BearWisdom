use super::*;
use crate::db::Database;

/// Insert the minimal rows needed for flow tests.
fn seed_flow(db: &Database) {
    let conn = db.conn();

    // Two files: a TypeScript frontend and a C# backend.
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES
         ('src/api/client.ts',   'h1', 'typescript', 0),
         ('src/CatalogController.cs', 'h2', 'csharp', 0)",
        [],
    )
    .unwrap();

    let ts_id: i64 = conn
        .query_row(
            "SELECT id FROM files WHERE path = 'src/api/client.ts'",
            [],
            |r| r.get(0),
        )
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
    assert_eq!(steps.len(), 2);
    let first = &steps[0];
    assert_eq!(first.file_path, "src/api/client.ts");
    assert_eq!(
        first.target_file_path.as_deref(),
        Some("src/CatalogController.cs")
    );
    assert_eq!(first.target_line, Some(42));
    assert_eq!(first.edge_type, "http_call");
    assert!(first.paired);

    let second = &steps[1];
    assert_eq!(second.parent_edge_id, Some(first.edge_id));
    assert_eq!(second.file_path, "src/CatalogController.cs");
    assert_eq!(second.line, Some(42));
    assert_eq!(second.target_line, Some(80));
}

#[test]
fn trace_reaches_downstream_hops() {
    let db = Database::open_in_memory().unwrap();
    seed_flow(&db);

    let steps = trace_flow(&db, "src/api/client.ts", 15, 3).unwrap();

    assert_eq!(steps.len(), 2);
    assert_eq!(steps[1].target_symbol.as_deref(), Some("LoadFromDb"));
}

#[test]
fn trace_does_not_splice_an_unrelated_edge_from_the_same_file() {
    let db = Database::open_in_memory().unwrap();
    seed_flow(&db);
    let conn = db.conn();
    let cs_id: i64 = conn
        .query_row(
            "SELECT id FROM files WHERE path = 'src/CatalogController.cs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO flow_edges (
            source_file_id, source_line, source_symbol, source_language,
            target_file_id, target_line, target_symbol, target_language,
            edge_type, confidence
         ) VALUES (?1, 999, 'Unrelated', 'csharp',
                   ?1, 1000, 'AlsoUnrelated', 'csharp', 'calls', 1.0)",
        [cs_id],
    )
    .unwrap();

    let steps = trace_flow(&db, "src/api/client.ts", 15, 3).unwrap();
    assert_eq!(steps.len(), 2);
    assert!(steps.iter().all(|step| step.line != Some(999)));
}

#[test]
fn trace_preserves_a_single_ended_observation_as_incomplete() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('src/client.ts', 'h', 'typescript', 0)",
        [],
    )
    .unwrap();
    let file_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO flow_edges (
            source_file_id, source_line, source_symbol, source_language,
            edge_type, protocol, url_pattern, confidence
         ) VALUES (?1, 7, 'load', 'typescript',
                   'http_call', 'http', '/missing', 0.4)",
        [file_id],
    )
    .unwrap();

    let steps = trace_flow(&db, "src/client.ts", 7, 3).unwrap();
    assert_eq!(steps.len(), 1);
    assert!(!steps[0].paired);
    assert!(steps[0].target_file_path.is_none());
    assert_eq!(steps[0].url_pattern.as_deref(), Some("/missing"));
}

#[test]
fn reverse_trace_follows_exact_endpoint_locations() {
    let db = Database::open_in_memory().unwrap();
    seed_flow(&db);

    let steps = trace_flow_reverse(&db, "src/CatalogController.cs", 80, 3).unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].symbol.as_deref(), Some("GetCatalog"));
    assert_eq!(steps[1].symbol.as_deref(), Some("fetchCatalog"));
    assert_eq!(steps[1].target_symbol.as_deref(), Some("GetCatalog"));
    assert_eq!(steps[1].parent_edge_id, Some(steps[0].edge_id));
}

#[test]
fn trace_cycle_does_not_repeat_edges() {
    let db = Database::open_in_memory().unwrap();
    seed_flow(&db);
    let conn = db.conn();
    let ts_id: i64 = conn
        .query_row(
            "SELECT id FROM files WHERE path = 'src/api/client.ts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let cs_id: i64 = conn
        .query_row(
            "SELECT id FROM files WHERE path = 'src/CatalogController.cs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    conn.execute(
        "INSERT INTO flow_edges (
            source_file_id, source_line, source_symbol, source_language,
            target_file_id, target_line, target_symbol, target_language,
            edge_type, confidence
         ) VALUES (?1, 80, 'LoadFromDb', 'csharp',
                   ?2, 15, 'fetchCatalog', 'typescript', 'calls', 1.0)",
        rusqlite::params![cs_id, ts_id],
    )
    .unwrap();

    let steps = trace_flow(&db, "src/api/client.ts", 15, 20).unwrap();
    assert_eq!(steps.len(), 3);
    let unique = steps
        .iter()
        .map(|step| step.edge_id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(unique.len(), 3);
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

    let has_csharp_target = first
        .iter()
        .any(|s| s.target_language.as_deref() == Some("csharp"));
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
