use super::*;
use crate::db::Database;

fn open() -> Database {
    Database::open_in_memory().unwrap()
}

fn insert_file(db: &Database, path: &str, lang: &str) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed) \
             VALUES (?1, 'h', ?2, 0)",
            rusqlite::params![path, lang],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

fn insert_symbol(
    db: &Database,
    file_id: i64,
    name: &str,
    qname: &str,
    kind: &str,
    line: u32,
    visibility: Option<&str>,
) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility) \
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
            rusqlite::params![file_id, name, qname, kind, line, visibility],
        )
        .unwrap();
    db.conn().last_insert_rowid()
}

#[test]
fn rebuild_seeds_main_contributor() {
    let db = open();
    let f = insert_file(&db, "src/main.rs", "rust");
    let main_id = insert_symbol(&db, f, "main", "main", "function", 1, None);
    let _other = insert_symbol(&db, f, "helper", "helper", "function", 10, Some("private"));

    rebuild_entry_points(&db).unwrap();
    let ids = load_entry_point_ids_for_exclusion(&db).unwrap();
    assert!(ids.contains(&main_id), "main must be an entry point");
    assert_eq!(ids.len(), 1, "only main should be present, got ids={ids:?}");
}

#[test]
fn rebuild_seeds_routes_contributor() {
    let db = open();
    let f = insert_file(&db, "Controllers.cs", "csharp");
    let handler = insert_symbol(&db, f, "Get", "Api.Users.Get", "method", 5, Some("public"));

    db.conn()
        .execute(
            "INSERT INTO routes (file_id, symbol_id, http_method, route_template, line) \
             VALUES (?1, ?2, 'GET', '/users', 5)",
            rusqlite::params![f, handler],
        )
        .unwrap();

    rebuild_entry_points(&db).unwrap();
    let ids = load_entry_point_ids_for_exclusion(&db).unwrap();
    assert!(ids.contains(&handler));
}

#[test]
fn rebuild_seeds_flow_edges_event_and_di() {
    let db = open();
    let f = insert_file(&db, "Handler.cs", "csharp");
    let event = insert_symbol(&db, f, "OnEvent", "App.OnEvent", "method", 10, Some("public"));
    let di = insert_symbol(&db, f, "Bind", "App.Bind", "method", 20, Some("public"));

    db.conn()
        .execute(
            "INSERT INTO flow_edges (source_file_id, target_file_id, target_line, edge_type) \
             VALUES (?1, ?1, 10, 'event_handler')",
            [f],
        )
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO flow_edges (source_file_id, target_file_id, target_line, edge_type) \
             VALUES (?1, ?1, 20, 'di_binding')",
            [f],
        )
        .unwrap();

    rebuild_entry_points(&db).unwrap();
    let ids = load_entry_point_ids_for_exclusion(&db).unwrap();
    assert!(ids.contains(&event));
    assert!(ids.contains(&di));
}

#[test]
fn exported_api_requires_publishable_package() {
    let db = open();
    db.conn()
        .execute(
            "INSERT INTO packages (name, path, kind, declared_name, is_publishable) \
             VALUES ('pub', 'crates/pub', 'cargo', 'mypkg', 1)",
            [],
        )
        .unwrap();
    let pub_pkg = db.conn().last_insert_rowid();
    db.conn()
        .execute(
            "INSERT INTO packages (name, path, kind, declared_name, is_publishable) \
             VALUES ('priv', 'crates/priv', 'cargo', 'internal', 0)",
            [],
        )
        .unwrap();
    let priv_pkg = db.conn().last_insert_rowid();

    let f_pub = {
        db.conn()
            .execute(
                "INSERT INTO files (path, hash, language, last_indexed, package_id) \
                 VALUES ('crates/pub/lib.rs', 'h', 'rust', 0, ?1)",
                [pub_pkg],
            )
            .unwrap();
        db.conn().last_insert_rowid()
    };
    let f_priv = {
        db.conn()
            .execute(
                "INSERT INTO files (path, hash, language, last_indexed, package_id) \
                 VALUES ('crates/priv/lib.rs', 'h', 'rust', 0, ?1)",
                [priv_pkg],
            )
            .unwrap();
        db.conn().last_insert_rowid()
    };

    let api = insert_symbol(&db, f_pub, "api", "mypkg::api", "function", 1, Some("public"));
    let internal = insert_symbol(&db, f_priv, "helper", "internal::helper", "function", 1, Some("public"));

    rebuild_entry_points(&db).unwrap();
    let ids = load_entry_point_ids_for_exclusion(&db).unwrap();
    assert!(ids.contains(&api), "public symbol in publishable pkg must be an entry point");
    assert!(
        !ids.contains(&internal),
        "public symbol in is_publishable=0 pkg must NOT be an entry point",
    );
}

#[test]
fn lifecycle_contributor_seeds_known_hooks() {
    let db = open();
    let f = insert_file(&db, "comp.ts", "typescript");
    let hook = insert_symbol(&db, f, "ngOnInit", "Comp.ngOnInit", "method", 1, Some("public"));
    let plain = insert_symbol(&db, f, "doStuff", "Comp.doStuff", "method", 10, Some("public"));

    rebuild_entry_points(&db).unwrap();
    let ids = load_entry_point_ids_for_exclusion(&db).unwrap();
    assert!(ids.contains(&hook));
    assert!(!ids.contains(&plain));
}

#[test]
fn test_functions_excluded_from_dead_code_exclusion_set() {
    // Test-function rows are present in the entry_points table but
    // load_entry_point_ids_for_exclusion filters them out — preserves the
    // historical separation between "tests aren't entry points for the
    // exclusion set" and "test files are excluded via is_test_file()".
    let db = open();
    let f = insert_file(&db, "src/foo.test.ts", "typescript");
    let t = insert_symbol(&db, f, "test_x", "test_x", "function", 1, None);

    rebuild_entry_points(&db).unwrap();
    let ids = load_entry_point_ids_for_exclusion(&db).unwrap();
    assert!(!ids.contains(&t), "test functions are not in the exclusion set");

    // But the test contributor DID emit a row — find_entry_points report sees it.
    let count: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM entry_points WHERE symbol_id = ?1 AND kind = 'test'",
            [t],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn report_includes_test_functions_but_not_lifecycle() {
    let db = open();
    let f = insert_file(&db, "src/foo.spec.ts", "typescript");
    insert_symbol(&db, f, "test_login", "test_login", "function", 1, None);
    insert_symbol(&db, f, "ngOnInit", "X.ngOnInit", "method", 10, Some("public"));

    let report = find_entry_points(&db).unwrap();
    assert!(report
        .entry_points
        .iter()
        .any(|e| matches!(e.entry_kind, EntryPointKind::TestFunction) && e.name == "test_login"));
    assert!(
        !report
            .entry_points
            .iter()
            .any(|e| matches!(e.entry_kind, EntryPointKind::LifecycleHook)),
        "report must not include lifecycle hooks (preserves v0 behavior)",
    );
}

#[test]
fn rebuild_is_idempotent_and_clears_stale_rows() {
    let db = open();
    let f = insert_file(&db, "src/main.rs", "rust");
    let m = insert_symbol(&db, f, "main", "main", "function", 1, None);

    rebuild_entry_points(&db).unwrap();
    let first: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM entry_points", [], |r| r.get(0))
        .unwrap();
    assert_eq!(first, 1);

    // Re-run — count unchanged.
    rebuild_entry_points(&db).unwrap();
    let second: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM entry_points", [], |r| r.get(0))
        .unwrap();
    assert_eq!(second, 1);

    // Delete the symbol — rebuild must drop the stale row.
    db.conn()
        .execute("DELETE FROM symbols WHERE id = ?1", [m])
        .unwrap();
    rebuild_entry_points(&db).unwrap();
    let third: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM entry_points", [], |r| r.get(0))
        .unwrap();
    assert_eq!(third, 0);
}

/// Phase 6 success criterion: an `export class Foo { bar() {} }` should
/// anchor `bar` as a reachability root even when `bar`'s own visibility
/// is NULL (the TS extractor doesn't tag class members public by default).
/// Without this transitive scope_path match, every method on every
/// exported class looks dead in the dead-code report.
#[test]
fn exported_class_anchors_its_methods() {
    let db = open();
    db.conn()
        .execute(
            "INSERT INTO packages (name, path, kind, declared_name, is_publishable) \
             VALUES ('lib', 'pkg', 'npm', '@org/lib', 1)",
            [],
        )
        .unwrap();
    let pkg = db.conn().last_insert_rowid();
    db.conn()
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed, package_id) \
             VALUES ('pkg/index.ts', 'h', 'typescript', 0, ?1)",
            [pkg],
        )
        .unwrap();
    let f = db.conn().last_insert_rowid();

    // Exported class `Foo` (visibility=public from the export keyword).
    db.conn()
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility, origin) \
             VALUES (?1, 'Foo', 'Foo', 'class', 1, 0, 'public', 'internal')",
            [f],
        )
        .unwrap();

    // Methods on Foo — class-default visibility (NULL) and explicit private.
    db.conn()
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility, scope_path, origin) \
             VALUES (?1, 'doWork', 'Foo.doWork', 'method', 2, 0, NULL, 'Foo', 'internal')",
            [f],
        )
        .unwrap();
    let do_work = db.conn().last_insert_rowid();

    db.conn()
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility, scope_path, origin) \
             VALUES (?1, 'secret', 'Foo.secret', 'method', 3, 0, 'private', 'Foo', 'internal')",
            [f],
        )
        .unwrap();
    let secret = db.conn().last_insert_rowid();

    rebuild_entry_points(&db).unwrap();
    let ids = load_entry_point_ids_for_exclusion(&db).unwrap();
    assert!(
        ids.contains(&do_work),
        "method with NULL visibility on an exported class must be rooted"
    );
    assert!(
        !ids.contains(&secret),
        "explicitly `private` method must NOT be rooted even on an exported class"
    );
}

/// Phase 7: user-declared roots from `.bw/roots.json` anchor reachability
/// even when no static contributor would have caught them. This is the
/// escape valve for eval, framework auto-discovery via strings, and other
/// dispatch the analyzer can't see.
#[test]
fn user_roots_json_anchors_qname_matches() {
    let tmp = tempfile::TempDir::new().unwrap();
    let project_root = tmp.path();
    std::fs::create_dir(project_root.join(".bw")).unwrap();
    std::fs::create_dir(project_root.join(".bearwisdom")).unwrap();
    std::fs::write(
        project_root.join(".bw").join("roots.json"),
        r#"{"qnames": ["app::dyn_handler"], "names": ["plugin_*"], "globs": ["src/handlers/*.ts"]}"#,
    )
    .unwrap();

    let db_path = project_root.join(".bearwisdom").join("index.db");
    let db = crate::db::Database::open(&db_path).unwrap();

    // Three symbols matching the three pattern types.
    let f1 = insert_file(&db, "app/lib.rs", "rust");
    let dyn_handler = insert_symbol(&db, f1, "dyn_handler", "app::dyn_handler", "function", 1, None);
    let plugin_init = insert_symbol(&db, f1, "plugin_init", "app::plugin_init", "function", 2, None);
    let plain = insert_symbol(&db, f1, "plain_fn", "app::plain_fn", "function", 3, None);

    let f2 = insert_file(&db, "src/handlers/users.ts", "typescript");
    let handler = insert_symbol(&db, f2, "handler", "users.handler", "function", 1, None);

    rebuild_entry_points(&db).unwrap();
    let ids = load_entry_point_ids_for_exclusion(&db).unwrap();
    assert!(ids.contains(&dyn_handler), "qname match must anchor");
    assert!(ids.contains(&plugin_init), "name glob match must anchor");
    assert!(ids.contains(&handler), "file glob match must anchor every symbol in matched files");
    assert!(!ids.contains(&plain), "non-matched symbol must NOT be anchored");
}

#[test]
fn user_roots_missing_file_silently_succeeds() {
    // An in-memory DB has no on-disk project root. The contributor must
    // bail out cleanly without producing rows or errors.
    let db = open();
    rebuild_entry_points(&db).unwrap();
    let ids = load_entry_point_ids_for_exclusion(&db).unwrap();
    assert!(ids.is_empty(), "in-memory DB must produce no user-roots rows");
}

/// A method on a NON-exported class (private to its module) must NOT
/// land as an entry point. The transitive match keys on the parent
/// class's visibility, not just on presence of a scope_path.
#[test]
fn private_class_methods_are_not_entry_points() {
    let db = open();
    db.conn()
        .execute(
            "INSERT INTO packages (name, path, kind, declared_name, is_publishable) \
             VALUES ('lib', 'pkg', 'npm', '@org/lib', 1)",
            [],
        )
        .unwrap();
    let pkg = db.conn().last_insert_rowid();
    db.conn()
        .execute(
            "INSERT INTO files (path, hash, language, last_indexed, package_id) \
             VALUES ('pkg/index.ts', 'h', 'typescript', 0, ?1)",
            [pkg],
        )
        .unwrap();
    let f = db.conn().last_insert_rowid();

    // Module-private class — no `export`, so visibility stays NULL after
    // the parent-walk fix in the TS extractor.
    db.conn()
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility, origin) \
             VALUES (?1, 'Helper', 'Helper', 'class', 1, 0, NULL, 'internal')",
            [f],
        )
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, visibility, scope_path, origin) \
             VALUES (?1, 'run', 'Helper.run', 'method', 2, 0, NULL, 'Helper', 'internal')",
            [f],
        )
        .unwrap();
    let run = db.conn().last_insert_rowid();

    rebuild_entry_points(&db).unwrap();
    let ids = load_entry_point_ids_for_exclusion(&db).unwrap();
    assert!(
        !ids.contains(&run),
        "method on a non-exported class must NOT auto-root"
    );
}
