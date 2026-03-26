use super::*;
use crate::db::Database;
use std::io::Write;
use tempfile::NamedTempFile;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_py_file(content: &str) -> NamedTempFile {
    let mut f = tempfile::Builder::new()
        .suffix(".py")
        .tempfile()
        .unwrap();
    write!(f, "{}", content).unwrap();
    f
}

fn insert_py_file(conn: &Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES (?1, 'h', 'python', 0)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn route_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM routes", [], |r| r.get(0))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Regex unit tests
// ---------------------------------------------------------------------------

#[test]
fn decorator_regex_matches_app_get() {
    let re = build_decorator_regex();
    let line = r#"@app.get("/users")"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "app");
    assert_eq!(&cap[2], "get");
    assert_eq!(&cap[3], "/users");
}

#[test]
fn decorator_regex_matches_router_post_with_path_param() {
    let re = build_decorator_regex();
    let line = r#"@router.post("/users/{user_id}")"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "router");
    assert_eq!(&cap[2], "post");
    assert_eq!(&cap[3], "/users/{user_id}");
}

#[test]
fn decorator_regex_matches_all_http_methods() {
    let re = build_decorator_regex();
    for method in &["get", "post", "put", "delete", "patch", "head", "options"] {
        let line = format!(r#"@app.{method}("/test")"#);
        assert!(re.is_match(&line), "should match @app.{method}");
    }
}

#[test]
fn decorator_regex_does_not_match_plain_function() {
    let re = build_decorator_regex();
    assert!(!re.is_match("def my_handler(request):"));
}

#[test]
fn apirouter_regex_captures_variable_and_prefix() {
    let re = build_apirouter_regex();
    let line = r#"router = APIRouter(prefix="/users")"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "router");
    assert_eq!(&cap[2], "/users");
}

#[test]
fn apirouter_regex_handles_extra_kwargs() {
    let re = build_apirouter_regex();
    let line = r#"items_router = APIRouter(prefix="/items", tags=["items"])"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "items_router");
    assert_eq!(&cap[2], "/items");
}

#[test]
fn include_router_regex_captures_prefix() {
    let re = build_include_router_regex();
    let line = r#"app.include_router(router, prefix="/api/v1")"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "router");
    assert_eq!(&cap[2], "/api/v1");
}

#[test]
fn join_prefix_produces_correct_paths() {
    assert_eq!(join_prefix("/users", "/items"), "/users/items");
    assert_eq!(join_prefix("", "/items"), "/items");
    assert_eq!(join_prefix("/users", ""), "/users");
    assert_eq!(join_prefix("/users/", "/items"), "/users/items");
    assert_eq!(join_prefix("/users", "items"), "/users/items");
}

// ---------------------------------------------------------------------------
// Integration tests: @app.get — simplest case, no prefix
// ---------------------------------------------------------------------------

#[test]
fn app_get_route_is_inserted() {
    let db = Database::open_in_memory().unwrap();
    let conn = &db.conn;

    let source = r#"
from fastapi import FastAPI
app = FastAPI()

@app.get("/users")
async def list_users():
    return []
"#;

    let py_file = make_py_file(source);
    let root = py_file.path().parent().unwrap();
    let file_name = py_file.path().file_name().unwrap().to_str().unwrap();
    insert_py_file(conn, file_name);

    let count = connect(conn, root).unwrap();
    assert_eq!(count, 1, "expected one route inserted");
    assert_eq!(route_count(conn), 1);

    let (method, template, resolved): (String, String, String) = conn
        .query_row(
            "SELECT http_method, route_template, resolved_route FROM routes LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();

    assert_eq!(method, "GET");
    assert_eq!(template, "/users");
    assert_eq!(resolved, "/users");
}

// ---------------------------------------------------------------------------
// Integration tests: @router.get with APIRouter prefix
// ---------------------------------------------------------------------------

#[test]
fn router_get_with_prefix_resolves_combined_route() {
    let db = Database::open_in_memory().unwrap();
    let conn = &db.conn;

    let source = r#"
from fastapi import APIRouter
router = APIRouter(prefix="/users")

@router.get("/{user_id}")
async def get_user(user_id: int):
    pass
"#;

    let py_file = make_py_file(source);
    let root = py_file.path().parent().unwrap();
    let file_name = py_file.path().file_name().unwrap().to_str().unwrap();
    insert_py_file(conn, file_name);

    let count = connect(conn, root).unwrap();
    assert_eq!(count, 1);

    let (template, resolved): (String, String) = conn
        .query_row(
            "SELECT route_template, resolved_route FROM routes LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();

    // template stays as declared on the decorator
    assert_eq!(template, "/{user_id}");
    // resolved_route should incorporate the router prefix
    assert_eq!(resolved, "/users/{user_id}");
}

// ---------------------------------------------------------------------------
// Integration tests: parameterized routes
// ---------------------------------------------------------------------------

#[test]
fn parameterized_route_is_stored_correctly() {
    let db = Database::open_in_memory().unwrap();
    let conn = &db.conn;

    let source = r#"
from fastapi import FastAPI
app = FastAPI()

@app.delete("/items/{item_id}")
async def delete_item(item_id: int):
    pass
"#;

    let py_file = make_py_file(source);
    let root = py_file.path().parent().unwrap();
    let file_name = py_file.path().file_name().unwrap().to_str().unwrap();
    insert_py_file(conn, file_name);

    let count = connect(conn, root).unwrap();
    assert_eq!(count, 1);

    let (method, template): (String, String) = conn
        .query_row(
            "SELECT http_method, route_template FROM routes LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();

    assert_eq!(method, "DELETE");
    assert_eq!(template, "/items/{item_id}");
}

// ---------------------------------------------------------------------------
// Integration tests: include_router mount prefix
// ---------------------------------------------------------------------------

#[test]
fn include_router_prefix_combined_with_apirouter_prefix() {
    let db = Database::open_in_memory().unwrap();
    let conn = &db.conn;

    // In a single file: declare APIRouter with prefix, then mount with additional prefix
    let source = r#"
from fastapi import FastAPI, APIRouter
app = FastAPI()
router = APIRouter(prefix="/users")
app.include_router(router, prefix="/api/v1")

@router.get("/")
async def list_users():
    return []
"#;

    let py_file = make_py_file(source);
    let root = py_file.path().parent().unwrap();
    let file_name = py_file.path().file_name().unwrap().to_str().unwrap();
    insert_py_file(conn, file_name);

    connect(conn, root).unwrap();

    let resolved: String = conn
        .query_row(
            "SELECT resolved_route FROM routes LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();

    // Route "/" with prefix "/api/v1/users" resolves to the prefix root.
    // join_prefix strips the trailing slash when the path portion is empty.
    assert_eq!(resolved, "/api/v1/users");
}

// ---------------------------------------------------------------------------
// Integration tests: multiple routes in one file
// ---------------------------------------------------------------------------

#[test]
fn multiple_routes_in_same_file_are_all_inserted() {
    let db = Database::open_in_memory().unwrap();
    let conn = &db.conn;

    let source = r#"
from fastapi import FastAPI
app = FastAPI()

@app.get("/items")
async def list_items():
    return []

@app.post("/items")
async def create_item():
    pass

@app.get("/items/{item_id}")
async def get_item(item_id: int):
    pass
"#;

    let py_file = make_py_file(source);
    let root = py_file.path().parent().unwrap();
    let file_name = py_file.path().file_name().unwrap().to_str().unwrap();
    insert_py_file(conn, file_name);

    let count = connect(conn, root).unwrap();
    assert_eq!(count, 3, "expected three routes");
}

// ---------------------------------------------------------------------------
// Integration tests: empty project succeeds with zero routes
// ---------------------------------------------------------------------------

#[test]
fn connect_on_empty_project_returns_zero() {
    let db = Database::open_in_memory().unwrap();
    let dir = tempfile::TempDir::new().unwrap();
    let count = connect(&db.conn, dir.path()).unwrap();
    assert_eq!(count, 0);
}
