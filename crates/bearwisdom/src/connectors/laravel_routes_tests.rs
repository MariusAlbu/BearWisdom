use super::*;
use crate::db::Database;
use std::io::Write;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_php_route_file(content: &str) -> tempfile::NamedTempFile {
    let mut f = tempfile::Builder::new()
        .prefix("routes")
        .suffix(".php")
        .tempfile()
        .unwrap();
    write!(f, "{}", content).unwrap();
    f
}

fn insert_php_file(conn: &Connection, path: &str) -> i64 {
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES (?1, 'h', 'php', 0)",
        [path],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn routes_for_method(conn: &Connection, method: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT resolved_route FROM routes WHERE http_method = ?1 ORDER BY resolved_route")
        .unwrap();
    stmt.query_map([method], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

// ---------------------------------------------------------------------------
// Regex unit tests
// ---------------------------------------------------------------------------

#[test]
fn explicit_route_regex_matches_get() {
    let re = build_explicit_route_regex();
    let line = r#"Route::get('/users', [UserController::class, 'index']);"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(cap[1].to_uppercase(), "GET");
    assert_eq!(&cap[2], "/users");
}

#[test]
fn explicit_route_regex_matches_post_double_quotes() {
    let re = build_explicit_route_regex();
    let line = r#"Route::post("/users", [UserController::class, "store"]);"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(cap[1].to_uppercase(), "POST");
    assert_eq!(&cap[2], "/users");
}

#[test]
fn explicit_route_regex_matches_legacy_at_syntax() {
    let re = build_explicit_route_regex();
    let line = r#"    Route::get('/users', 'UserController@index');"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[2], "/users");
}

#[test]
fn explicit_route_regex_matches_all_verbs() {
    let re = build_explicit_route_regex();
    for verb in &["get", "post", "put", "patch", "delete", "options", "any"] {
        let line = format!(r#"Route::{verb}('/x', handler);"#);
        let cap = re.captures(&line).unwrap();
        assert_eq!(cap[1].to_lowercase(), *verb, "verb mismatch for {verb}");
    }
}

#[test]
fn resource_regex_matches_resource_call() {
    let re = build_resource_regex();
    let line = r#"Route::resource('users', UserController::class);"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "users");
}

#[test]
fn api_resource_regex_matches_api_resource_call() {
    let re = build_api_resource_regex();
    let line = r#"Route::apiResource('photos', PhotoController::class);"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "photos");
}

#[test]
fn prefix_regex_matches_fluent_prefix() {
    let re = build_prefix_regex();
    let line = r#"Route::prefix('api/v1')->group(function () {"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "api/v1");
}

#[test]
fn prefix_regex_matches_array_prefix() {
    let re = build_prefix_regex();
    let line = r#"Route::group(['prefix' => 'admin'], function () {"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "admin");
}

#[test]
fn join_prefix_stack_empty_gives_empty() {
    assert_eq!(join_prefix_stack(&[]), "");
}

#[test]
fn join_prefix_stack_single() {
    assert_eq!(
        join_prefix_stack(&["api".to_string()]),
        "/api"
    );
}

#[test]
fn join_prefix_stack_nested() {
    assert_eq!(
        join_prefix_stack(&["api".to_string(), "v2".to_string()]),
        "/api/v2"
    );
}

// ---------------------------------------------------------------------------
// Integration test 1: explicit routes (GET, POST, legacy @syntax)
// ---------------------------------------------------------------------------

#[test]
fn explicit_routes_inserted_correctly() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let source = r#"<?php

use App\Http\Controllers\UserController;

Route::get('/users', [UserController::class, 'index']);
Route::post('/users', [UserController::class, 'store']);
Route::get('/users/{id}', 'UserController@show');
Route::put('/users/{id}', [UserController::class, 'update']);
Route::delete('/users/{id}', [UserController::class, 'destroy']);
"#;

    let file = make_php_route_file(source);
    let root = file.path().parent().unwrap();
    let name = format!("routes/{}", file.path().file_name().unwrap().to_str().unwrap());

    // Write the file at a path matching the DB record.
    let routes_dir = root.join("routes");
    std::fs::create_dir_all(&routes_dir).unwrap();
    std::fs::write(routes_dir.join(file.path().file_name().unwrap()), source).unwrap();

    insert_php_file(conn, &name);

    let count = connect(conn, root).unwrap();
    assert_eq!(count, 5, "Expected 5 explicit routes");

    assert_eq!(routes_for_method(conn, "GET"), vec!["/users", "/users/{id}"]);
    assert_eq!(routes_for_method(conn, "POST"), vec!["/users"]);
    assert_eq!(routes_for_method(conn, "PUT"), vec!["/users/{id}"]);
    assert_eq!(routes_for_method(conn, "DELETE"), vec!["/users/{id}"]);
}

// ---------------------------------------------------------------------------
// Integration test 2: Route::resource expands to 7 routes, apiResource to 5
// ---------------------------------------------------------------------------

#[test]
fn resource_and_api_resource_expansion() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let source = r#"<?php

Route::resource('photos', PhotoController::class);
Route::apiResource('comments', CommentController::class);
"#;

    let file = make_php_route_file(source);
    let root = file.path().parent().unwrap();
    let name = format!("routes/{}", file.path().file_name().unwrap().to_str().unwrap());

    let routes_dir = root.join("routes");
    std::fs::create_dir_all(&routes_dir).unwrap();
    std::fs::write(routes_dir.join(file.path().file_name().unwrap()), source).unwrap();

    insert_php_file(conn, &name);

    let count = connect(conn, root).unwrap();
    assert_eq!(count, 12, "resource(7) + apiResource(5) = 12");

    // resource expands: index, create, store, show, edit, update, destroy
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM routes WHERE route_template LIKE 'photos%'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(total, 7, "photos resource should have 7 rows");

    // apiResource expands: index, store, show, update, destroy (no create/edit)
    let api_total: i64 = conn
        .query_row("SELECT COUNT(*) FROM routes WHERE route_template LIKE 'comments%'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(api_total, 5, "comments apiResource should have 5 rows");

    // Verify create and edit are absent for apiResource.
    let no_create: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM routes WHERE route_template = 'comments/create'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(no_create, 0, "apiResource must not emit a /create route");

    let no_edit: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM routes WHERE route_template LIKE 'comments%/edit'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(no_edit, 0, "apiResource must not emit a /{{id}}/edit route");
}

// ---------------------------------------------------------------------------
// Integration test 3: Route::prefix groups (including nested)
// ---------------------------------------------------------------------------

#[test]
fn prefixed_group_routes_resolve_correctly() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    // Outer prefix 'api', inner prefix 'v1'.
    let source = r#"<?php

Route::prefix('api')->group(function () {
    Route::prefix('v1')->group(function () {
        Route::get('/users', [UserController::class, 'index']);
        Route::post('/users', [UserController::class, 'store']);
    });
    Route::get('/status', [StatusController::class, 'index']);
});
Route::get('/health', [HealthController::class, 'check']);
"#;

    let file = make_php_route_file(source);
    let root = file.path().parent().unwrap();
    let name = format!("routes/{}", file.path().file_name().unwrap().to_str().unwrap());

    let routes_dir = root.join("routes");
    std::fs::create_dir_all(&routes_dir).unwrap();
    std::fs::write(routes_dir.join(file.path().file_name().unwrap()), source).unwrap();

    insert_php_file(conn, &name);

    let count = connect(conn, root).unwrap();
    assert_eq!(count, 4, "Expected 4 routes total");

    // Nested prefix should be resolved.
    let resolved: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT resolved_route FROM routes ORDER BY resolved_route")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };

    assert!(
        resolved.contains(&"/api/v1/users".to_string()),
        "Expected /api/v1/users, got: {resolved:?}"
    );
    assert!(
        resolved.contains(&"/api/status".to_string()),
        "Expected /api/status, got: {resolved:?}"
    );
    assert!(
        resolved.contains(&"/health".to_string()),
        "Expected /health (no prefix), got: {resolved:?}"
    );
}

// ---------------------------------------------------------------------------
// Integration test 4: Route::group with array prefix syntax
// ---------------------------------------------------------------------------

#[test]
fn group_array_prefix_syntax() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let source = r#"<?php

Route::group(['prefix' => 'admin'], function () {
    Route::get('/dashboard', [AdminController::class, 'index']);
});
"#;

    let file = make_php_route_file(source);
    let root = file.path().parent().unwrap();
    let name = format!("routes/{}", file.path().file_name().unwrap().to_str().unwrap());

    let routes_dir = root.join("routes");
    std::fs::create_dir_all(&routes_dir).unwrap();
    std::fs::write(routes_dir.join(file.path().file_name().unwrap()), source).unwrap();

    insert_php_file(conn, &name);

    connect(conn, root).unwrap();

    let resolved: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT resolved_route FROM routes")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };

    assert!(
        resolved.contains(&"/admin/dashboard".to_string()),
        "Expected /admin/dashboard, got: {resolved:?}"
    );
}

// ---------------------------------------------------------------------------
// Integration test 5: connect on empty project succeeds with zero routes
// ---------------------------------------------------------------------------

#[test]
fn connect_empty_project_returns_zero() {
    let db = Database::open_in_memory().unwrap();
    let dir = tempfile::TempDir::new().unwrap();
    let count = connect(db.conn(), dir.path()).unwrap();
    assert_eq!(count, 0);
}

// ---------------------------------------------------------------------------
// Integration test 6: Route::match inserts one row per method
// ---------------------------------------------------------------------------

#[test]
fn match_route_inserts_per_method() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let source = r#"<?php

Route::match(['get', 'post'], '/contact', [ContactController::class, 'handle']);
"#;

    let file = make_php_route_file(source);
    let root = file.path().parent().unwrap();
    let name = format!("routes/{}", file.path().file_name().unwrap().to_str().unwrap());

    let routes_dir = root.join("routes");
    std::fs::create_dir_all(&routes_dir).unwrap();
    std::fs::write(routes_dir.join(file.path().file_name().unwrap()), source).unwrap();

    insert_php_file(conn, &name);

    let count = connect(conn, root).unwrap();
    assert_eq!(count, 2, "match(['get','post']) should insert 2 rows");

    assert_eq!(routes_for_method(conn, "GET"), vec!["/contact"]);
    assert_eq!(routes_for_method(conn, "POST"), vec!["/contact"]);
}
