//! Integration tests for SCIP index import.
//!
//! These tests differ from the unit tests in `bridge/scip_tests.rs` in that
//! they use `full_index` against the C# service fixture to populate the
//! database, then construct SCIP protobufs whose file paths and line numbers
//! are derived from real indexed content.  This exercises the path
//! normalisation logic and the symbol-resolution path end-to-end.
//!
//! The unit tests cover the low-level DB helpers and edge-upsert logic in
//! isolation; these tests cover the contract between the indexer output and
//! the SCIP importer input.

use bearwisdom::bridge::scip::{
    Document, Metadata, Occurrence, ProtocolVersion, ScipIndex, TextEncoding, ToolInfo,
    SYMBOL_ROLE_DEFINITION, SYMBOL_ROLE_READ_ACCESS,
};
use bearwisdom::{full_index, import_scip};
use bearwisdom_tests::TestProject;
use prost::Message;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Encode a `ScipIndex` to a temp file and return (TempDir, path to .scip file).
/// The TempDir must be kept alive for the duration of the test.
fn write_scip_index(index: &ScipIndex) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.scip");
    let mut buf = Vec::new();
    index.encode(&mut buf).unwrap();
    std::fs::write(&path, &buf).unwrap();
    (dir, path)
}

/// Build the standard SCIP metadata pointing at `project_root`.
fn metadata_for(project_root: &std::path::Path) -> Metadata {
    Metadata {
        version: ProtocolVersion::UnspecifiedProtocolVersion as i32,
        tool_info: Some(ToolInfo {
            name: "test-scip-emitter".into(),
            version: "0.0.1".into(),
            arguments: vec![],
        }),
        project_root: format!(
            "file:///{}",
            project_root.to_string_lossy().replace('\\', "/")
        ),
        text_document_encoding: TextEncoding::Utf8 as i32,
    }
}

/// Count all edges in the database.
fn edge_count(db: &bearwisdom::Database) -> i64 {
    db.conn()
        .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
        .unwrap()
}

/// Count edges of a specific `kind`.
fn edge_count_by_kind(db: &bearwisdom::Database, kind: &str) -> i64 {
    db.conn()
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE kind = ?1",
            rusqlite::params![kind],
            |r| r.get(0),
        )
        .unwrap()
}

/// Return all (source_line, confidence) pairs for scip_ref edges between
/// the two symbols whose `name` columns match the given names.
fn scip_ref_edges(
    db: &bearwisdom::Database,
    source_name: &str,
    target_name: &str,
) -> Vec<(Option<i32>, f64)> {
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT e.source_line, e.confidence
             FROM edges e
             JOIN symbols s  ON s.id  = e.source_id
             JOIN symbols t  ON t.id  = e.target_id
             WHERE s.name = ?1 AND t.name = ?2 AND e.kind = 'scip_ref'",
        )
        .unwrap();

    stmt.query_map(rusqlite::params![source_name, target_name], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })
    .unwrap()
    .map(|r| r.unwrap())
    .collect()
}

// ---------------------------------------------------------------------------
// Helper: resolve the normalised file path as stored in the DB for a
// fixture file so SCIP can reference it correctly.
// ---------------------------------------------------------------------------

/// Return the relative path stored in `files.path` for a given fixture file.
/// On Windows the indexer normalises to forward slashes; we use whatever
/// is actually in the DB so SCIP path matching succeeds.
fn db_file_path(db: &bearwisdom::Database, suffix: &str) -> String {
    // Allow both / and \ variants; return whichever exists in the DB.
    for sep in ["/", "\\"] {
        let pattern = format!("%{}{}", sep, suffix.replace('/', sep));
        if let Ok(p) = db.conn().query_row(
            "SELECT path FROM files WHERE path LIKE ?1 LIMIT 1",
            rusqlite::params![pattern],
            |r| r.get::<_, String>(0),
        ) {
            return p;
        }
    }
    // Fall back to the suffix itself.
    suffix.to_string()
}

/// Return the 0-based start line for a symbol with the given `name`.
fn symbol_line(db: &bearwisdom::Database, name: &str) -> i32 {
    db.conn()
        .query_row(
            "SELECT line FROM symbols WHERE name = ?1 LIMIT 1",
            rusqlite::params![name],
            |r| r.get(0),
        )
        .unwrap_or(0)
}

/// Return the 0-based end line for a symbol (falls back to start line if NULL).
fn symbol_end_line(db: &bearwisdom::Database, name: &str) -> i32 {
    db.conn()
        .query_row(
            "SELECT COALESCE(end_line, line) FROM symbols WHERE name = ?1 LIMIT 1",
            rusqlite::params![name],
            |r| r.get(0),
        )
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// test_scip_import_creates_edges
// ---------------------------------------------------------------------------

/// Index the C# fixture, construct a SCIP index that describes one of the
/// real files, and verify that edges with `kind='scip_ref'` at confidence 1.0
/// are created.
///
/// The SCIP protobuf is built to have:
///   - `ProductService` defined at its actual line in ProductService.cs
///   - `IProductRepository` defined at its actual line in IProductRepository.cs
///   - a reference occurrence pointing from inside ProductService to IProductRepository
///
/// Because both symbols are in different files we use a two-document SCIP
/// index so that both definitions are resolved before the reference pass.
#[test]
fn test_scip_import_creates_edges() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None).unwrap();

    // Identify the paths and lines the indexer actually recorded.
    let svc_path = db_file_path(&db, "ProductService.cs");
    let repo_path = db_file_path(&db, "IProductRepository.cs");

    let svc_line = symbol_line(&db, "ProductService");
    let svc_end = symbol_end_line(&db, "ProductService");
    let repo_line = symbol_line(&db, "IProductRepository");
    let repo_end = symbol_end_line(&db, "IProductRepository");

    // Reference line: somewhere inside ProductService's span (but not at the
    // definition start itself, to avoid being treated as a definition).
    let ref_line = if svc_end > svc_line + 1 {
        svc_line + 1
    } else {
        svc_line
    };

    let scip_svc_sym =
        format!("scip-dotnet nuget MyApp 1.0 MyApp.Services/ProductService#.");
    let scip_repo_sym =
        format!("scip-dotnet nuget MyApp 1.0 MyApp.Repositories/IProductRepository#.");

    let index = ScipIndex {
        metadata: Some(metadata_for(project.path())),
        documents: vec![
            // Document 1: ProductService.cs — defines ProductService and
            // references IProductRepository.
            Document {
                relative_path: svc_path.clone(),
                language: "csharp".into(),
                text: String::new(),
                occurrences: vec![
                    // Definition of ProductService at its actual line.
                    Occurrence {
                        range: vec![svc_line, 0, svc_end, 0],
                        symbol: scip_svc_sym.clone(),
                        symbol_roles: SYMBOL_ROLE_DEFINITION,
                        ..Default::default()
                    },
                    // Reference to IProductRepository from inside the service.
                    Occurrence {
                        range: vec![ref_line, 4, 22],
                        symbol: scip_repo_sym.clone(),
                        symbol_roles: SYMBOL_ROLE_READ_ACCESS,
                        ..Default::default()
                    },
                ],
                symbols: vec![],
            },
            // Document 2: IProductRepository.cs — defines IProductRepository.
            Document {
                relative_path: repo_path.clone(),
                language: "csharp".into(),
                text: String::new(),
                occurrences: vec![Occurrence {
                    range: vec![repo_line, 0, repo_end, 0],
                    symbol: scip_repo_sym.clone(),
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    ..Default::default()
                }],
                symbols: vec![],
            },
        ],
        external_symbols: vec![],
    };

    let (_tmp, scip_path) = write_scip_index(&index);
    let stats = import_scip(&db, &scip_path, project.path()).unwrap();

    // At least both definition sites must have been resolved.
    assert!(
        stats.symbols_matched >= 2,
        "expected at least 2 symbols matched, got {}",
        stats.symbols_matched
    );

    // At least one scip_ref edge must have been created.
    assert!(
        stats.edges_created >= 1,
        "expected at least one edge, got {}",
        stats.edges_created
    );

    // The edge must have confidence = 1.0.
    let edges = scip_ref_edges(&db, "ProductService", "IProductRepository");
    assert!(
        !edges.is_empty(),
        "no scip_ref edge found from ProductService to IProductRepository"
    );
    for (_, conf) in &edges {
        assert!(
            (conf - 1.0_f64).abs() < f64::EPSILON,
            "scip_ref edge must have confidence 1.0, got {conf}"
        );
    }

    // Every scip_ref edge in the DB must also have confidence = 1.0.
    let any_low: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM edges WHERE kind='scip_ref' AND confidence < 1.0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        any_low, 0,
        "all scip_ref edges must be at confidence 1.0 after import"
    );
}

// ---------------------------------------------------------------------------
// test_scip_import_upgrades_confidence
// ---------------------------------------------------------------------------

/// Index the fixture, pre-insert a low-confidence edge between two known
/// symbols, then import SCIP data that confirms the edge.  Verify the
/// confidence is upgraded to 1.0.
#[test]
fn test_scip_import_upgrades_confidence() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None).unwrap();

    let svc_path = db_file_path(&db, "ProductService.cs");
    let repo_path = db_file_path(&db, "IProductRepository.cs");

    let svc_line = symbol_line(&db, "ProductService");
    let svc_end = symbol_end_line(&db, "ProductService");
    let repo_line = symbol_line(&db, "IProductRepository");
    let repo_end = symbol_end_line(&db, "IProductRepository");

    // The reference line sits one line into ProductService's body.
    let ref_line = if svc_end > svc_line + 1 {
        svc_line + 1
    } else {
        svc_line
    };

    // Fetch IDs for the two symbols so we can pre-insert the edge.
    let svc_id: i64 = db
        .conn()
        .query_row(
            "SELECT id FROM symbols WHERE name = 'ProductService' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let repo_id: i64 = db
        .conn()
        .query_row(
            "SELECT id FROM symbols WHERE name = 'IProductRepository' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();

    // Pre-insert a low-confidence edge (0.3).
    db.conn()
        .execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
             VALUES (?1, ?2, 'scip_ref', ?3, 0.3)",
            rusqlite::params![svc_id, repo_id, ref_line],
        )
        .unwrap();

    // Confirm edge is at 0.3 before import.
    let pre_conf: f64 = db
        .conn()
        .query_row(
            "SELECT confidence FROM edges WHERE source_id=?1 AND target_id=?2",
            rusqlite::params![svc_id, repo_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        (pre_conf - 0.3_f64).abs() < 1e-9,
        "pre-condition: confidence should be 0.3, got {pre_conf}"
    );

    // Build SCIP that covers both definitions + the reference.
    let scip_svc_sym = "scip-dotnet nuget MyApp 1.0 MyApp.Services/ProductService#.".to_string();
    let scip_repo_sym =
        "scip-dotnet nuget MyApp 1.0 MyApp.Repositories/IProductRepository#.".to_string();

    let index = ScipIndex {
        metadata: Some(metadata_for(project.path())),
        documents: vec![
            Document {
                relative_path: svc_path,
                language: "csharp".into(),
                text: String::new(),
                occurrences: vec![
                    Occurrence {
                        range: vec![svc_line, 0, svc_end, 0],
                        symbol: scip_svc_sym.clone(),
                        symbol_roles: SYMBOL_ROLE_DEFINITION,
                        ..Default::default()
                    },
                    Occurrence {
                        range: vec![ref_line, 4, 22],
                        symbol: scip_repo_sym.clone(),
                        symbol_roles: SYMBOL_ROLE_READ_ACCESS,
                        ..Default::default()
                    },
                ],
                symbols: vec![],
            },
            Document {
                relative_path: repo_path,
                language: "csharp".into(),
                text: String::new(),
                occurrences: vec![Occurrence {
                    range: vec![repo_line, 0, repo_end, 0],
                    symbol: scip_repo_sym.clone(),
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    ..Default::default()
                }],
                symbols: vec![],
            },
        ],
        external_symbols: vec![],
    };

    let (_tmp, scip_path) = write_scip_index(&index);
    let stats = import_scip(&db, &scip_path, project.path()).unwrap();

    // Either the per-edge upsert or the bulk upgrade must have fired.
    assert!(
        stats.edges_upgraded >= 1,
        "expected at least one upgraded edge, got {}",
        stats.edges_upgraded
    );

    // Confirm edge is now at 1.0.
    let post_conf: f64 = db
        .conn()
        .query_row(
            "SELECT confidence FROM edges WHERE source_id=?1 AND target_id=?2",
            rusqlite::params![svc_id, repo_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        (post_conf - 1.0_f64).abs() < f64::EPSILON,
        "confidence should be upgraded to 1.0, got {post_conf}"
    );
}

// ---------------------------------------------------------------------------
// test_scip_import_idempotent
// ---------------------------------------------------------------------------

/// Import the same SCIP data twice against a real-indexed fixture and verify
/// the edge count does not grow on the second import.
#[test]
fn test_scip_import_idempotent() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None).unwrap();

    let svc_path = db_file_path(&db, "ProductService.cs");
    let repo_path = db_file_path(&db, "IProductRepository.cs");

    let svc_line = symbol_line(&db, "ProductService");
    let svc_end = symbol_end_line(&db, "ProductService");
    let repo_line = symbol_line(&db, "IProductRepository");
    let repo_end = symbol_end_line(&db, "IProductRepository");
    let ref_line = if svc_end > svc_line + 1 { svc_line + 1 } else { svc_line };

    let scip_svc_sym = "scip-dotnet nuget MyApp 1.0 MyApp.Services/ProductService#.".to_string();
    let scip_repo_sym =
        "scip-dotnet nuget MyApp 1.0 MyApp.Repositories/IProductRepository#.".to_string();

    let index = ScipIndex {
        metadata: Some(metadata_for(project.path())),
        documents: vec![
            Document {
                relative_path: svc_path,
                language: "csharp".into(),
                text: String::new(),
                occurrences: vec![
                    Occurrence {
                        range: vec![svc_line, 0, svc_end, 0],
                        symbol: scip_svc_sym.clone(),
                        symbol_roles: SYMBOL_ROLE_DEFINITION,
                        ..Default::default()
                    },
                    Occurrence {
                        range: vec![ref_line, 4, 22],
                        symbol: scip_repo_sym.clone(),
                        symbol_roles: SYMBOL_ROLE_READ_ACCESS,
                        ..Default::default()
                    },
                ],
                symbols: vec![],
            },
            Document {
                relative_path: repo_path,
                language: "csharp".into(),
                text: String::new(),
                occurrences: vec![Occurrence {
                    range: vec![repo_line, 0, repo_end, 0],
                    symbol: scip_repo_sym,
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    ..Default::default()
                }],
                symbols: vec![],
            },
        ],
        external_symbols: vec![],
    };

    let (_tmp, scip_path) = write_scip_index(&index);

    import_scip(&db, &scip_path, project.path()).unwrap();
    let edges_after_first = edge_count_by_kind(&db, "scip_ref");

    let stats2 = import_scip(&db, &scip_path, project.path()).unwrap();
    let edges_after_second = edge_count_by_kind(&db, "scip_ref");

    assert_eq!(
        edges_after_first, edges_after_second,
        "scip_ref edge count must not grow on second import"
    );
    assert_eq!(
        stats2.edges_created, 0,
        "second import must create zero new edges"
    );
}

// ---------------------------------------------------------------------------
// test_scip_import_empty_index
// ---------------------------------------------------------------------------

/// An index with metadata but no documents must not crash and must report
/// zero edges created.
#[test]
fn test_scip_import_empty_index() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None).unwrap();

    let edges_before = edge_count(&db);

    let index = ScipIndex {
        metadata: Some(metadata_for(project.path())),
        documents: vec![],
        external_symbols: vec![],
    };

    let (_tmp, scip_path) = write_scip_index(&index);
    let stats = import_scip(&db, &scip_path, project.path()).unwrap();

    assert_eq!(stats.documents_processed, 0);
    assert_eq!(stats.edges_created, 0);
    assert_eq!(stats.edges_upgraded, 0);
    assert_eq!(
        edge_count(&db),
        edges_before,
        "no edges should be added for an empty SCIP index"
    );
}

// ---------------------------------------------------------------------------
// test_scip_import_unmatched_file_path
// ---------------------------------------------------------------------------

/// A SCIP document whose relative_path does not match any file in the DB
/// must be silently skipped without error.
#[test]
fn test_scip_import_unmatched_file_path() {
    let project = TestProject::csharp_service();
    let mut db = TestProject::in_memory_db();
    full_index(&mut db, project.path(), None, None).unwrap();

    let index = ScipIndex {
        metadata: Some(metadata_for(project.path())),
        documents: vec![Document {
            relative_path: "does/not/exist/Phantom.cs".into(),
            language: "csharp".into(),
            text: String::new(),
            occurrences: vec![
                Occurrence {
                    range: vec![0, 0, 10, 0],
                    symbol: "scip-dotnet nuget MyApp 1.0 MyApp/Phantom#.".into(),
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    ..Default::default()
                },
                Occurrence {
                    range: vec![5, 4, 10],
                    symbol: "scip-dotnet nuget MyApp 1.0 MyApp/Phantom#.".into(),
                    symbol_roles: SYMBOL_ROLE_READ_ACCESS,
                    ..Default::default()
                },
            ],
            symbols: vec![],
        }],
        external_symbols: vec![],
    };

    let edges_before = edge_count(&db);

    let (_tmp, scip_path) = write_scip_index(&index);
    let stats = import_scip(&db, &scip_path, project.path()).unwrap();

    assert_eq!(stats.documents_processed, 0, "unmatched file should be skipped");
    assert_eq!(stats.edges_created, 0);
    assert_eq!(
        edge_count(&db),
        edges_before,
        "no new edges should be created when the SCIP document file does not exist in the DB"
    );
}
