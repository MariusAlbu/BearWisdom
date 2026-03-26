use super::*;
use crate::db::Database;
use prost::Message;

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

/// Create an in-memory DB pre-seeded with one file and two symbols.
///
/// Returns `(db, file_id, caller_id, callee_id)`.
/// - caller: function at line 0, spans lines 0–9.
/// - callee: function at line 20, spans lines 20–29.
fn seed_db() -> (Database, i64, i64, i64) {
    let db = Database::open_in_memory().unwrap();

    db.conn
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed)
             VALUES ('src/app.ts', 'abc', 'typescript', 0)",
            [],
        )
        .unwrap();
    let file_id = db.conn.last_insert_rowid();

    db.conn
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, end_line)
             VALUES (?1, 'caller', 'app.caller', 'function', 0, 0, 9)",
            rusqlite::params![file_id],
        )
        .unwrap();
    let caller_id = db.conn.last_insert_rowid();

    db.conn
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, end_line)
             VALUES (?1, 'callee', 'app.callee', 'function', 20, 0, 29)",
            rusqlite::params![file_id],
        )
        .unwrap();
    let callee_id = db.conn.last_insert_rowid();

    (db, file_id, caller_id, callee_id)
}

/// Build a minimal ScipIndex in memory and encode it to bytes.
///
/// The index describes `src/app.ts`:
///   - `scip-typescript npm app 1.0 src/app.ts/caller().`  is defined at line 0.
///   - `scip-typescript npm app 1.0 src/app.ts/callee().`  is defined at line 20.
///   - `scip-typescript npm app 1.0 src/app.ts/callee().`  is referenced at line 5
///     (inside `caller`).
fn build_scip_bytes(doc_path: &str) -> Vec<u8> {
    let index = ScipIndex {
        metadata: Some(Metadata {
            version: ProtocolVersion::UnspecifiedProtocolVersion as i32,
            tool_info: Some(ToolInfo {
                name: "scip-typescript".into(),
                version: "0.3.0".into(),
                arguments: vec![],
            }),
            project_root: "file:///workspace".into(),
            text_document_encoding: TextEncoding::Utf8 as i32,
        }),
        documents: vec![Document {
            relative_path: doc_path.into(),
            language: "typescript".into(),
            text: String::new(),
            occurrences: vec![
                // Definition of `caller` at line 0.
                Occurrence {
                    range: vec![0, 0, 9, 0],
                    symbol: "scip-typescript npm app 1.0 src/app.ts/caller().".into(),
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    override_documentation: vec![],
                    syntax_kind: SyntaxKind::IdentifierFunctionDefinition as i32,
                    diagnostics: vec![],
                },
                // Definition of `callee` at line 20.
                Occurrence {
                    range: vec![20, 0, 29, 0],
                    symbol: "scip-typescript npm app 1.0 src/app.ts/callee().".into(),
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    override_documentation: vec![],
                    syntax_kind: SyntaxKind::IdentifierFunctionDefinition as i32,
                    diagnostics: vec![],
                },
                // Reference to `callee` from within `caller` at line 5.
                Occurrence {
                    range: vec![5, 4, 10],
                    symbol: "scip-typescript npm app 1.0 src/app.ts/callee().".into(),
                    symbol_roles: SYMBOL_ROLE_READ_ACCESS,
                    override_documentation: vec![],
                    syntax_kind: SyntaxKind::IdentifierFunction as i32,
                    diagnostics: vec![],
                },
            ],
            symbols: vec![],
        }],
        external_symbols: vec![],
    };

    let mut buf = Vec::new();
    index.encode(&mut buf).unwrap();
    buf
}

// -----------------------------------------------------------------------
// Unit tests: helper functions
// -----------------------------------------------------------------------

#[test]
fn scip_range_start_line_three_element() {
    // [startLine, startChar, endChar]
    assert_eq!(scip_range_start_line(&[5, 4, 10]), 5);
}

#[test]
fn scip_range_start_line_four_element() {
    // [startLine, startChar, endLine, endChar]
    assert_eq!(scip_range_start_line(&[12, 0, 15, 0]), 12);
}

#[test]
fn scip_range_start_line_empty() {
    assert_eq!(scip_range_start_line(&[]), 0);
}

#[test]
fn scip_symbol_to_qualified_name_typescript() {
    let sym = "scip-typescript npm my-pkg 1.0 src/foo.ts/MyClass#method().";
    let qname = scip_symbol_to_qualified_name(sym);
    // Should strip the SCIP preamble (4 tokens) and normalise separators.
    assert_eq!(qname, "src.foo.ts.MyClass.method");
}

#[test]
fn scip_symbol_to_qualified_name_dotnet() {
    let sym = "scip-dotnet nuget Microsoft.Extensions.DI 7.0 Microsoft.Extensions.DependencyInjection.ServiceCollection#AddSingleton().";
    let qname = scip_symbol_to_qualified_name(sym);
    assert_eq!(
        qname,
        "Microsoft.Extensions.DependencyInjection.ServiceCollection.AddSingleton"
    );
}

#[test]
fn scip_symbol_to_qualified_name_too_few_tokens() {
    let sym = "short symbol";
    let qname = scip_symbol_to_qualified_name(sym);
    // Falls back to whole string.
    assert_eq!(qname, sym);
}

#[test]
fn normalise_doc_path_strips_project_root() {
    let root = Path::new("/workspace/myproject");
    let result = normalise_doc_path("src/index.ts", root, "");
    assert_eq!(result, "src/index.ts");
}

#[test]
fn normalise_doc_path_strips_absolute_prefix() {
    let root = Path::new("/workspace/myproject");
    let result = normalise_doc_path("/workspace/myproject/src/index.ts", root, "");
    assert_eq!(result, "src/index.ts");
}

#[test]
fn normalise_doc_path_strips_uri_and_scip_root() {
    let root = Path::new("/other/path");
    let result = normalise_doc_path(
        "file:///workspace/src/app.ts",
        root,
        "file:///workspace",
    );
    assert_eq!(result, "src/app.ts");
}

#[test]
fn normalise_doc_path_strips_leading_dot_slash() {
    let root = Path::new("/workspace");
    let result = normalise_doc_path("./src/app.ts", root, "");
    assert_eq!(result, "src/app.ts");
}

// -----------------------------------------------------------------------
// Unit tests: DB helpers
// -----------------------------------------------------------------------

#[test]
fn lookup_file_id_found_and_not_found() {
    let (db, file_id, _, _) = seed_db();
    let found = lookup_file_id(&db.conn, "src/app.ts").unwrap();
    assert_eq!(found, Some(file_id));

    let missing = lookup_file_id(&db.conn, "nonexistent.ts").unwrap();
    assert!(missing.is_none());
}

#[test]
fn lookup_symbol_by_file_and_line_exact() {
    let (db, file_id, caller_id, callee_id) = seed_db();

    let found = lookup_symbol_by_file_and_line(&db.conn, file_id, 0).unwrap();
    assert_eq!(found, Some(caller_id));

    let found2 = lookup_symbol_by_file_and_line(&db.conn, file_id, 20).unwrap();
    assert_eq!(found2, Some(callee_id));
}

#[test]
fn lookup_narrowest_symbol_at_line_inside_span() {
    let (db, file_id, caller_id, callee_id) = seed_db();

    // Line 5 is inside caller (0–9).
    let found = lookup_narrowest_symbol_at_line(&db.conn, file_id, 5).unwrap();
    assert_eq!(found, Some(caller_id));

    // Line 25 is inside callee (20–29).
    let found2 = lookup_narrowest_symbol_at_line(&db.conn, file_id, 25).unwrap();
    assert_eq!(found2, Some(callee_id));

    // Line 50 is outside both.
    let outside = lookup_narrowest_symbol_at_line(&db.conn, file_id, 50).unwrap();
    assert!(outside.is_none());
}

#[test]
fn lookup_symbol_by_qualified_name_exact_and_suffix() {
    let (db, _, caller_id, _) = seed_db();

    // Exact match.
    let found = lookup_symbol_by_qualified_name(&db.conn, "app.caller").unwrap();
    assert_eq!(found, Some(caller_id));

    // Suffix match — SCIP descriptor may have package prefix not in DB.
    let found2 = lookup_symbol_by_qualified_name(&db.conn, "caller").unwrap();
    assert_eq!(found2, Some(caller_id));

    // Not found.
    let missing = lookup_symbol_by_qualified_name(&db.conn, "totally.unknown").unwrap();
    assert!(missing.is_none());
}

// -----------------------------------------------------------------------
// Unit tests: edge upsert
// -----------------------------------------------------------------------

#[test]
fn upsert_scip_edge_creates_new_edge() {
    let (db, _, caller_id, callee_id) = seed_db();

    let change = upsert_scip_edge(&db.conn, caller_id, callee_id, 5).unwrap();
    assert_eq!(change, EdgeChange::Created);

    let conf: f64 = db
        .conn
        .query_row(
            "SELECT confidence FROM edges
             WHERE source_id = ?1 AND target_id = ?2",
            rusqlite::params![caller_id, callee_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!((conf - 1.0).abs() < f64::EPSILON);
}

#[test]
fn upsert_scip_edge_idempotent() {
    let (db, _, caller_id, callee_id) = seed_db();

    upsert_scip_edge(&db.conn, caller_id, callee_id, 5).unwrap();
    // Second call should be a no-op.
    let change = upsert_scip_edge(&db.conn, caller_id, callee_id, 5).unwrap();
    assert_eq!(change, EdgeChange::Unchanged);

    let count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "idempotent: still only one edge");
}

#[test]
fn upsert_scip_edge_upgrades_low_confidence() {
    let (db, _, caller_id, callee_id) = seed_db();

    // Pre-insert a tree-sitter edge at 0.6.
    db.conn
        .execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
             VALUES (?1, ?2, 'scip_ref', 5, 0.6)",
            rusqlite::params![caller_id, callee_id],
        )
        .unwrap();

    let change = upsert_scip_edge(&db.conn, caller_id, callee_id, 5).unwrap();
    assert_eq!(change, EdgeChange::Upgraded);

    let conf: f64 = db
        .conn
        .query_row(
            "SELECT confidence FROM edges
             WHERE source_id = ?1 AND target_id = ?2",
            rusqlite::params![caller_id, callee_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!((conf - 1.0).abs() < f64::EPSILON, "should be upgraded to 1.0");
}

// -----------------------------------------------------------------------
// Integration test: import_scip end-to-end
// -----------------------------------------------------------------------

#[test]
fn import_scip_creates_edge_from_reference_occurrence() {
    let dir = tempfile::TempDir::new().unwrap();
    let scip_path = dir.path().join("index.scip");

    let (db, _file_id, caller_id, callee_id) = seed_db();

    // Write the encoded SCIP index to a temp file.
    let bytes = build_scip_bytes("src/app.ts");
    std::fs::write(&scip_path, &bytes).unwrap();

    let project_root = Path::new("/workspace");
    let stats = import_scip(&db, &scip_path, project_root).unwrap();

    assert_eq!(stats.documents_processed, 1);
    // Two definition occurrences should match (caller and callee).
    assert_eq!(stats.symbols_matched, 2, "both definitions should match");
    assert_eq!(stats.edges_created, 1, "one reference edge expected");
    assert_eq!(stats.edges_upgraded, 0);
    assert_eq!(stats.symbols_unmatched, 0);

    // Verify the actual edge exists with confidence 1.0.
    let conf: f64 = db
        .conn
        .query_row(
            "SELECT confidence FROM edges
             WHERE source_id = ?1 AND target_id = ?2 AND kind = 'scip_ref'",
            rusqlite::params![caller_id, callee_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        (conf - 1.0).abs() < f64::EPSILON,
        "edge confidence should be 1.0, got {conf}"
    );
}

#[test]
fn import_scip_is_idempotent() {
    let dir = tempfile::TempDir::new().unwrap();
    let scip_path = dir.path().join("index.scip");
    let bytes = build_scip_bytes("src/app.ts");
    std::fs::write(&scip_path, &bytes).unwrap();

    let (db, _, _, _) = seed_db();
    let project_root = Path::new("/workspace");

    let stats1 = import_scip(&db, &scip_path, project_root).unwrap();
    let stats2 = import_scip(&db, &scip_path, project_root).unwrap();

    // Edge count in DB should be the same after both runs.
    let count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "idempotent: still one edge after two imports");

    // Second run should create nothing and upgrade nothing (edge already at 1.0).
    assert_eq!(stats2.edges_created, 0, "second run should create no new edges");
    // Any upgraded count from bulk_upgrade is fine to be 0 on the second run too.
    let _ = stats1; // used — suppress lint
}

#[test]
fn import_scip_skips_unknown_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let scip_path = dir.path().join("index.scip");

    // Build SCIP index referencing a file that is NOT in the DB.
    let bytes = build_scip_bytes("src/unknown_file.ts");
    std::fs::write(&scip_path, &bytes).unwrap();

    let (db, _, _, _) = seed_db();
    let project_root = Path::new("/workspace");

    let stats = import_scip(&db, &scip_path, project_root).unwrap();

    // Document is not found in DB — should process 0 documents.
    assert_eq!(stats.documents_processed, 0);
    assert_eq!(stats.edges_created, 0);

    let count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn import_scip_upgrades_preexisting_low_confidence_edge() {
    let dir = tempfile::TempDir::new().unwrap();
    let scip_path = dir.path().join("index.scip");
    let bytes = build_scip_bytes("src/app.ts");
    std::fs::write(&scip_path, &bytes).unwrap();

    let (db, _, caller_id, callee_id) = seed_db();

    // Pre-seed a tree-sitter edge at 0.7 confidence.
    db.conn
        .execute(
            "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
             VALUES (?1, ?2, 'scip_ref', 5, 0.7)",
            rusqlite::params![caller_id, callee_id],
        )
        .unwrap();

    let project_root = Path::new("/workspace");
    let stats = import_scip(&db, &scip_path, project_root).unwrap();

    assert_eq!(stats.edges_upgraded, 1, "should upgrade the pre-existing edge");
    assert_eq!(stats.edges_created, 0);

    let conf: f64 = db
        .conn
        .query_row(
            "SELECT confidence FROM edges
             WHERE source_id = ?1 AND target_id = ?2",
            rusqlite::params![caller_id, callee_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!((conf - 1.0).abs() < f64::EPSILON);
}

#[test]
fn import_scip_handles_relationship_edges() {
    let dir = tempfile::TempDir::new().unwrap();
    let scip_path = dir.path().join("index.scip");

    let (db, _, caller_id, callee_id) = seed_db();

    // Build a SCIP index that has a SymbolInformation relationship
    // (caller implements callee — contrived but tests the path).
    let caller_sym = "scip-typescript npm app 1.0 src/app.ts/caller().".to_string();
    let callee_sym = "scip-typescript npm app 1.0 src/app.ts/callee().".to_string();

    let index = ScipIndex {
        metadata: None,
        documents: vec![Document {
            relative_path: "src/app.ts".into(),
            language: "typescript".into(),
            text: String::new(),
            occurrences: vec![
                Occurrence {
                    range: vec![0, 0, 9, 0],
                    symbol: caller_sym.clone(),
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                },
                Occurrence {
                    range: vec![20, 0, 29, 0],
                    symbol: callee_sym.clone(),
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                },
            ],
            symbols: vec![SymbolInformation {
                symbol: caller_sym.clone(),
                documentation: vec![],
                relationships: vec![Relationship {
                    symbol: callee_sym.clone(),
                    is_reference: false,
                    is_implementation: true,
                    is_type_definition: false,
                    is_definition: false,
                }],
                kind: SymbolKindScip::Function as i32,
                display_name: "caller".into(),
                signature_documentation: None,
                enclosing_symbol: String::new(),
            }],
        }],
        external_symbols: vec![],
    };

    let mut buf = Vec::new();
    index.encode(&mut buf).unwrap();
    std::fs::write(&scip_path, &buf).unwrap();

    let stats = import_scip(&db, &scip_path, Path::new("/workspace")).unwrap();

    assert_eq!(stats.documents_processed, 1);
    assert_eq!(stats.edges_created, 1, "relationship edge should be created");

    let kind: String = db
        .conn
        .query_row(
            "SELECT kind FROM edges WHERE source_id = ?1 AND target_id = ?2",
            rusqlite::params![caller_id, callee_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(kind, "implements");
}

#[test]
fn import_scip_no_self_edges() {
    let dir = tempfile::TempDir::new().unwrap();
    let scip_path = dir.path().join("index.scip");

    let (db, _, _, _) = seed_db();

    // Build a SCIP index where caller references itself.
    let sym = "scip-typescript npm app 1.0 src/app.ts/caller().".to_string();
    let index = ScipIndex {
        metadata: None,
        documents: vec![Document {
            relative_path: "src/app.ts".into(),
            language: "typescript".into(),
            text: String::new(),
            occurrences: vec![
                Occurrence {
                    range: vec![0, 0, 9, 0],
                    symbol: sym.clone(),
                    symbol_roles: SYMBOL_ROLE_DEFINITION,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                },
                // Reference to itself at line 5 (still inside caller's span).
                Occurrence {
                    range: vec![5, 0, 10],
                    symbol: sym.clone(),
                    symbol_roles: SYMBOL_ROLE_READ_ACCESS,
                    override_documentation: vec![],
                    syntax_kind: 0,
                    diagnostics: vec![],
                },
            ],
            symbols: vec![],
        }],
        external_symbols: vec![],
    };

    let mut buf = Vec::new();
    index.encode(&mut buf).unwrap();
    std::fs::write(&scip_path, &buf).unwrap();

    let stats = import_scip(&db, &scip_path, Path::new("/workspace")).unwrap();

    assert_eq!(stats.edges_created, 0, "self-edge must be suppressed");
    let count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn import_scip_bad_file_returns_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let bad_path = dir.path().join("garbage.scip");
    std::fs::write(&bad_path, b"this is not a valid protobuf").unwrap();

    let (db, _, _, _) = seed_db();
    let result = import_scip(&db, &bad_path, Path::new("/workspace"));
    assert!(result.is_err(), "corrupt SCIP file should return Err");
}

#[test]
fn import_scip_missing_file_returns_error() {
    let (db, _, _, _) = seed_db();
    let result = import_scip(
        &db,
        Path::new("/no/such/file.scip"),
        Path::new("/workspace"),
    );
    assert!(result.is_err(), "missing SCIP file should return Err");
}
