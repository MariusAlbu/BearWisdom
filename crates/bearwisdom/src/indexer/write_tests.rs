use super::*;
use crate::type_checker::core::types::TypeArena;
use crate::types::{ExtractedSymbol, PackageInfo, SymbolKind, Visibility};

#[test]
fn declared_name_round_trips_through_db() {
    let db = Database::open_in_memory().unwrap();
    let packages = vec![
        PackageInfo {
            id: None,
            name: "web".into(),
            path: "web".into(),
            kind: Some("npm".into()),
            manifest: Some("web/package.json".into()),
            declared_name: Some("@myorg/web".into()),
            is_publishable: true,
        },
        PackageInfo {
            id: None,
            name: "shared".into(),
            path: "shared".into(),
            kind: Some("npm".into()),
            manifest: Some("shared/package.json".into()),
            declared_name: Some("@myorg/shared".into()),
            is_publishable: true,
        },
    ];

    let written = write_packages(&db, &packages).unwrap();
    assert_eq!(written.len(), 2);

    let loaded = load_packages_from_db(&db).unwrap();
    let web = loaded.iter().find(|p| p.name == "web").expect("web");
    let shared = loaded.iter().find(|p| p.name == "shared").expect("shared");
    assert_eq!(web.declared_name.as_deref(), Some("@myorg/web"));
    assert_eq!(shared.declared_name.as_deref(), Some("@myorg/shared"));
}

#[test]
fn declared_name_nullable_when_absent() {
    let db = Database::open_in_memory().unwrap();
    let packages = vec![PackageInfo {
        id: None,
        name: "legacy".into(),
        path: "legacy".into(),
        kind: None,
        manifest: None,
        declared_name: None,
        is_publishable: true,
    }];
    write_packages(&db, &packages).unwrap();
    let loaded = load_packages_from_db(&db).unwrap();
    assert_eq!(loaded.len(), 1);
    assert!(loaded[0].declared_name.is_none());
}

fn pf(path: &str) -> crate::types::ParsedFile {
    crate::types::ParsedFile {
        path: path.to_string(),
        language: "rust".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols: Vec::new(),
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    }
}

/// A root package with `path = ""` must claim every file in the project
/// when no deeper-prefix package matches. The empty-path entry sorts last
/// (length desc), so it only wins when no proper-prefix package matched.
#[test]
fn assign_package_ids_root_package_claims_all_files() {
    let packages = vec![PackageInfo {
        id: Some(7),
        name: "root".into(),
        path: String::new(),
        kind: Some("cargo".into()),
        manifest: Some("Cargo.toml".into()),
        declared_name: Some("root".into()),
        is_publishable: true,
    }];

    let mut parsed = vec![pf("src/lib.rs"), pf("examples/demo.rs"), pf("Cargo.toml")];

    assign_package_ids(&mut parsed, &packages);
    for p in &parsed {
        assert_eq!(p.package_id, Some(7), "root package should claim {}", p.path);
    }
}

/// Deeper-prefix packages must beat the root package even though the root
/// entry's `starts_with("")` always returns true. Sort order ensures the
/// real prefix is tried first; the empty-path entry is the fallback.
#[test]
fn assign_package_ids_deeper_prefix_beats_root() {
    let packages = vec![
        PackageInfo {
            id: Some(1),
            name: "root".into(),
            path: String::new(),
            kind: Some("npm".into()),
            manifest: Some("package.json".into()),
            declared_name: Some("root".into()),
            is_publishable: true,
        },
        PackageInfo {
            id: Some(2),
            name: "web".into(),
            path: "apps/web".into(),
            kind: Some("npm".into()),
            manifest: Some("apps/web/package.json".into()),
            declared_name: Some("@org/web".into()),
            is_publishable: true,
        },
    ];

    let mut parsed = vec![pf("apps/web/src/index.ts"), pf("tools/lint.ts")];

    assign_package_ids(&mut parsed, &packages);
    assert_eq!(parsed[0].package_id, Some(2), "web file picks the deeper package");
    assert_eq!(parsed[1].package_id, Some(1), "unrelated file falls through to root");
}

// ---------------------------------------------------------------------------
// Survivor-matching incremental write (SYMBOL-IDENTITY.md §4)
// ---------------------------------------------------------------------------

/// Build an `ExtractedSymbol` with the fields the write path reads.
fn esym(qname: &str, kind: SymbolKind, sig: Option<&str>, line: u32) -> ExtractedSymbol {
    ExtractedSymbol {
        name: qname.rsplit(['.', ':']).next().unwrap_or(qname).to_string(),
        qualified_name: qname.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: line,
        end_line: line,
        start_col: 0,
        end_col: 0,
        byte_offset: 0,
        signature: sig.map(str::to_string),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn pfile(path: &str, lang: &str, symbols: Vec<ExtractedSymbol>) -> crate::types::ParsedFile {
    let mut p = pf(path);
    p.language = lang.to_string();
    p.symbols = symbols;
    p
}

fn sym_id(db: &Database, qname: &str) -> Option<i64> {
    db.conn()
        .query_row(
            "SELECT id FROM symbols WHERE qualified_name = ?1",
            [qname],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
}

fn loc_count(db: &Database, sym_id: i64) -> i64 {
    db.conn()
        .query_row(
            "SELECT COUNT(*) FROM symbol_locations WHERE symbol_id = ?1",
            [sym_id],
            |r| r.get(0),
        )
        .unwrap()
}

fn insert_edge(db: &Database, source: i64, target: i64) {
    db.conn()
        .execute(
            "INSERT OR IGNORE INTO edges (source_id, target_id, kind, source_line, confidence)
             VALUES (?1, ?2, 'calls', 1, 1.0)",
            rusqlite::params![source, target],
        )
        .unwrap();
}

/// A body change keeps every key stable, so every id survives — the invariant
/// the whole refactor exists to provide.
#[test]
fn survivor_keeps_id_across_body_change() {
    let db = Database::open_in_memory().unwrap();
    let arena = TypeArena::new();

    let mut foo1 = esym("C.foo", SymbolKind::Method, Some("fn foo() {1}"), 2);
    foo1.parent_index = Some(0);
    let v1 = pfile("a.rs", "rust", vec![esym("C", SymbolKind::Class, Some("class C v1"), 1), foo1]);
    write_parsed_files_incremental(&db, std::slice::from_ref(&v1), Some(&arena)).unwrap();
    let foo_id = sym_id(&db, "C.foo").unwrap();
    let c_id = sym_id(&db, "C").unwrap();

    let mut foo2 = esym("C.foo", SymbolKind::Method, Some("fn foo() {2}"), 5);
    foo2.parent_index = Some(0);
    let v2 = pfile("a.rs", "rust", vec![esym("C", SymbolKind::Class, Some("class C v2"), 1), foo2]);
    write_parsed_files_incremental(&db, std::slice::from_ref(&v2), Some(&arena)).unwrap();

    assert_eq!(sym_id(&db, "C.foo"), Some(foo_id), "body change keeps the method id");
    assert_eq!(sym_id(&db, "C"), Some(c_id), "class id stable");

    // The survivor's mutable columns are refreshed from the new declaration.
    let line: i64 = db
        .conn()
        .query_row("SELECT line FROM symbols WHERE id = ?1", [foo_id], |r| r.get(0))
        .unwrap();
    assert_eq!(line, 5, "survivor position refreshed");
    let cid: Option<i64> = db
        .conn()
        .query_row("SELECT containing_id FROM symbols WHERE id = ?1", [foo_id], |r| r.get(0))
        .unwrap();
    assert_eq!(cid, Some(c_id), "containment edge re-pointed to the surviving parent");
}

/// A parameter-type change is a contract change: the key churns, so the old
/// symbol vanishes and a new one is inserted under a fresh id.
#[test]
fn param_change_replaces_symbol_and_reports_new_name() {
    let db = Database::open_in_memory().unwrap();
    let arena = TypeArena::new();

    let mut foo1 = esym("M.foo", SymbolKind::Function, Some("fn foo(x: int)"), 1);
    foo1.param_types = vec![arena.intern_type_str("int")];
    let v1 = pfile("a.rs", "rust", vec![foo1]);
    write_parsed_files_incremental(&db, std::slice::from_ref(&v1), Some(&arena)).unwrap();
    let id1 = sym_id(&db, "M.foo").unwrap();

    let mut foo2 = esym("M.foo", SymbolKind::Function, Some("fn foo(x: string)"), 1);
    foo2.param_types = vec![arena.intern_type_str("string")];
    let v2 = pfile("a.rs", "rust", vec![foo2]);
    let (_, _, report) =
        write_parsed_files_incremental(&db, std::slice::from_ref(&v2), Some(&arena)).unwrap();

    let id2 = sym_id(&db, "M.foo").unwrap();
    assert_ne!(id1, id2, "param-type change → new id");
    assert!(report.new_symbol_names.contains("foo"), "new key reported as a new name");
    let cnt: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM symbols WHERE qualified_name = 'M.foo'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cnt, 1, "old M.foo vanished, exactly one remains");
}

/// A mergeable symbol (namespace) declared in two files is one logical row with
/// two locations; dropping one declaration leaves the id and the other location.
#[test]
fn mergeable_namespace_shares_one_id_across_files() {
    let db = Database::open_in_memory().unwrap();
    let arena = TypeArena::new();

    let files = vec![
        pfile("a.cs", "csharp", vec![esym("App.Models", SymbolKind::Namespace, None, 1)]),
        pfile("b.cs", "csharp", vec![esym("App.Models", SymbolKind::Namespace, None, 1)]),
    ];
    write_parsed_files_incremental(&db, &files, Some(&arena)).unwrap();

    let cnt: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM symbols WHERE qualified_name = 'App.Models'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cnt, 1, "namespace collapses to one logical symbol");
    let id = sym_id(&db, "App.Models").unwrap();
    assert_eq!(loc_count(&db, id), 2, "declared in two files → two locations");

    // b.cs no longer declares it: location dropped, id stable (primary is a.cs).
    let b_empty = pfile("b.cs", "csharp", vec![]);
    write_parsed_files_incremental(&db, std::slice::from_ref(&b_empty), Some(&arena)).unwrap();
    assert_eq!(sym_id(&db, "App.Models"), Some(id), "id stable when one file drops it");
    assert_eq!(loc_count(&db, id), 1, "only a.cs's location remains");
}

/// When the PRIMARY file drops a mergeable symbol that still lives elsewhere,
/// the row is re-homed (primary promoted) rather than deleted — id stays stable.
#[test]
fn mergeable_rehomes_primary_when_primary_file_drops_it() {
    let db = Database::open_in_memory().unwrap();
    let arena = TypeArena::new();

    let files = vec![
        pfile("a.cs", "csharp", vec![esym("App.Svc", SymbolKind::Namespace, None, 3)]),
        pfile("b.cs", "csharp", vec![esym("App.Svc", SymbolKind::Namespace, None, 7)]),
    ];
    write_parsed_files_incremental(&db, &files, Some(&arena)).unwrap();
    let id = sym_id(&db, "App.Svc").unwrap();
    let file_a: i64 = db
        .conn()
        .query_row("SELECT id FROM files WHERE path = 'a.cs'", [], |r| r.get(0))
        .unwrap();
    let primary: i64 = db
        .conn()
        .query_row("SELECT file_id FROM symbols WHERE id = ?1", [id], |r| r.get(0))
        .unwrap();
    assert_eq!(primary, file_a, "a.cs is the primary site");

    let a_empty = pfile("a.cs", "csharp", vec![]);
    write_parsed_files_incremental(&db, std::slice::from_ref(&a_empty), Some(&arena)).unwrap();

    assert_eq!(sym_id(&db, "App.Svc"), Some(id), "id stable across re-home");
    let file_b: i64 = db
        .conn()
        .query_row("SELECT id FROM files WHERE path = 'b.cs'", [], |r| r.get(0))
        .unwrap();
    let primary_after: i64 = db
        .conn()
        .query_row("SELECT file_id FROM symbols WHERE id = ?1", [id], |r| r.get(0))
        .unwrap();
    assert_eq!(primary_after, file_b, "primary re-homed to the surviving file");
    assert_eq!(loc_count(&db, id), 1);
}

/// The narrowed blast radius: a vanished symbol reports its dependents, a
/// survivor's stale OUTGOING edges are cleared (body re-resolves), and INBOUND
/// edges to the survivor are preserved (consumers never re-resolve).
#[test]
fn vanished_reports_dependents_and_clears_only_outgoing() {
    let db = Database::open_in_memory().unwrap();
    let arena = TypeArena::new();

    let files = vec![
        pfile(
            "a.rs",
            "rust",
            vec![
                esym("M.foo", SymbolKind::Function, None, 1),
                esym("M.helper", SymbolKind::Function, None, 2),
            ],
        ),
        pfile("b.rs", "rust", vec![esym("N.bar", SymbolKind::Function, None, 1)]),
    ];
    write_parsed_files_incremental(&db, &files, Some(&arena)).unwrap();
    let foo = sym_id(&db, "M.foo").unwrap();
    let helper = sym_id(&db, "M.helper").unwrap();
    let bar = sym_id(&db, "N.bar").unwrap();

    insert_edge(&db, helper, bar); // survivor's outgoing edge (will be cleared)
    insert_edge(&db, bar, foo); // b depends on the vanishing foo
    insert_edge(&db, bar, helper); // b depends on the surviving helper (inbound)

    // Reparse a.rs without foo — foo vanishes, helper survives.
    let a2 = pfile("a.rs", "rust", vec![esym("M.helper", SymbolKind::Function, None, 2)]);
    let (_, _, report) =
        write_parsed_files_incremental(&db, std::slice::from_ref(&a2), Some(&arena)).unwrap();

    assert!(sym_id(&db, "M.foo").is_none(), "foo vanished");
    assert_eq!(sym_id(&db, "M.helper"), Some(helper), "helper id stable");
    assert!(
        report.vanished_dependent_paths.contains("b.rs"),
        "dependent of vanished foo reported: {:?}",
        report.vanished_dependent_paths
    );

    let helper_out: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM edges WHERE source_id = ?1", [helper], |r| r.get(0))
        .unwrap();
    assert_eq!(helper_out, 0, "survivor's outgoing edges cleared for re-resolution");
    let helper_in: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM edges WHERE target_id = ?1", [helper], |r| r.get(0))
        .unwrap();
    assert_eq!(helper_in, 1, "inbound edge to the survivor is preserved");
}
