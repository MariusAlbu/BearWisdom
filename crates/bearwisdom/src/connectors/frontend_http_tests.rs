use super::*;
use crate::db::Database;
use std::io::Write;
use tempfile::NamedTempFile;

// -----------------------------------------------------------------------
// Unit tests for helpers
// -----------------------------------------------------------------------

#[test]
fn normalise_url_strips_query_string() {
    assert_eq!(
        normalise_url_pattern("/api/items?page=1"),
        "/api/items"
    );
}

#[test]
fn normalise_url_replaces_template_literals() {
    assert_eq!(
        normalise_url_pattern("/api/items/${id}/details"),
        "/api/items/{param}/details"
    );
}

#[test]
fn extract_fetch_method_defaults_to_get() {
    assert_eq!(extract_fetch_method(r#"fetch("/api/items")"#), "GET");
}

#[test]
fn extract_fetch_method_finds_post() {
    assert_eq!(
        extract_fetch_method(r#"fetch("/api/items", { method: 'POST', body: JSON.stringify(data) })"#),
        "POST"
    );
}

// -----------------------------------------------------------------------
// Integration tests against in-memory DB + temp files
// -----------------------------------------------------------------------

fn make_db_with_route(method: &str, template: &str) -> Database {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('CatalogController.cs', 'h1', 'csharp', 0)",
        [],
    )
    .unwrap();
    let cs_file_id: i64 = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'GetItems', 'Catalog.GetItems', 'method', 10, 0)",
        [cs_file_id],
    )
    .unwrap();
    let sym_id: i64 = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO routes (file_id, symbol_id, http_method, route_template, resolved_route, line)
         VALUES (?1, ?2, ?3, ?4, ?4, 10)",
        rusqlite::params![cs_file_id, sym_id, method, template],
    )
    .unwrap();

    db
}

fn write_ts_file(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    write!(f, "{}", content).unwrap();
    f
}

#[test]
fn detect_fetch_get_call() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let ts_file = write_ts_file(r#"const data = await fetch("/api/catalog/items");"#);
    let root = ts_file.path().parent().unwrap();
    let file_name = ts_file.path().file_name().unwrap().to_str().unwrap();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'typescript', 0)",
        [file_name],
    )
    .unwrap();

    let calls = detect_http_calls(conn, root).unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].http_method, "GET");
    assert_eq!(calls[0].raw_url, "/api/catalog/items");
}

#[test]
fn detect_axios_post_call() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let ts_file =
        write_ts_file(r#"await axios.post("/api/orders", { body });"#);
    let root = ts_file.path().parent().unwrap();
    let file_name = ts_file.path().file_name().unwrap().to_str().unwrap();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'typescript', 0)",
        [file_name],
    )
    .unwrap();

    let calls = detect_http_calls(conn, root).unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].http_method, "POST");
    assert_eq!(calls[0].raw_url, "/api/orders");
}

#[test]
fn detect_template_literal_url() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let ts_file = write_ts_file(
        "const r = await fetch(`/api/items/${id}`);\n",
    );
    let root = ts_file.path().parent().unwrap();
    let file_name = ts_file.path().file_name().unwrap().to_str().unwrap();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'typescript', 0)",
        [file_name],
    )
    .unwrap();

    let calls = detect_http_calls(conn, root).unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url_pattern, "/api/items/{param}");
}

#[test]
fn match_calls_to_routes_inserts_flow_edge() {
    let db = make_db_with_route("GET", "/api/catalog/items");
    let conn = db.conn();

    // Insert a TS file.
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('src/client.ts', 'h2', 'typescript', 0)",
        [],
    )
    .unwrap();
    let ts_file_id: i64 = conn.last_insert_rowid();

    let calls = vec![DetectedHttpCall {
        file_id: ts_file_id,
        symbol_id: None,
        line: 5,
        http_method: "GET".into(),
        url_pattern: "/api/catalog/items".into(),
        raw_url: "/api/catalog/items".into(),
    }];

    let created = match_http_calls_to_routes(conn, &calls).unwrap();
    assert_eq!(created, 1, "Expected one flow_edge to be created");

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM flow_edges WHERE edge_type = 'http_call'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn match_calls_method_mismatch_creates_no_edge() {
    let db = make_db_with_route("POST", "/api/catalog/items");
    let conn = db.conn();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('src/client.ts', 'h2', 'typescript', 0)",
        [],
    )
    .unwrap();
    let ts_file_id: i64 = conn.last_insert_rowid();

    let calls = vec![DetectedHttpCall {
        file_id: ts_file_id,
        symbol_id: None,
        line: 5,
        http_method: "GET".into(), // route is POST — should not match
        url_pattern: "/api/catalog/items".into(),
        raw_url: "/api/catalog/items".into(),
    }];

    let created = match_http_calls_to_routes(conn, &calls).unwrap();
    assert_eq!(created, 0);
}

#[test]
fn match_calls_to_routes_with_path_param() {
    let db = make_db_with_route("GET", "/api/catalog/items/{id}");
    let conn = db.conn();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('src/client.ts', 'h2', 'typescript', 0)",
        [],
    )
    .unwrap();
    let ts_file_id: i64 = conn.last_insert_rowid();

    let calls = vec![DetectedHttpCall {
        file_id: ts_file_id,
        symbol_id: None,
        line: 7,
        http_method: "GET".into(),
        // Template literal collapsed to {param}
        url_pattern: "/api/catalog/items/{param}".into(),
        raw_url: "/api/catalog/items/${id}".into(),
    }];

    let created = match_http_calls_to_routes(conn, &calls).unwrap();
    assert_eq!(created, 1);
}

#[test]
fn no_routes_in_db_returns_zero() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('src/client.ts', 'h', 'typescript', 0)",
        [],
    )
    .unwrap();
    let ts_file_id: i64 = conn.last_insert_rowid();

    let calls = vec![DetectedHttpCall {
        file_id: ts_file_id,
        symbol_id: None,
        line: 1,
        http_method: "GET".into(),
        url_pattern: "/api/anything".into(),
        raw_url: "/api/anything".into(),
    }];

    let created = match_http_calls_to_routes(conn, &calls).unwrap();
    assert_eq!(created, 0);
}

// -----------------------------------------------------------------------
// Multi-language detection tests
// -----------------------------------------------------------------------

#[test]
fn detect_python_requests_get() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let mut f = tempfile::Builder::new().suffix(".py").tempfile().unwrap();
    write!(f, r#"response = requests.get("https://api.example.com/items")"#).unwrap();

    let root = f.path().parent().unwrap();
    let file_name = f.path().file_name().unwrap().to_str().unwrap();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'python', 0)",
        [file_name],
    )
    .unwrap();

    let calls = detect_http_calls_all_languages(conn, root).unwrap();
    let py_calls: Vec<_> = calls
        .iter()
        .filter(|c| c.raw_url.contains("api.example.com"))
        .collect();
    assert_eq!(py_calls.len(), 1);
    assert_eq!(py_calls[0].http_method, "GET");
}

#[test]
fn detect_go_http_post() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let mut f = tempfile::Builder::new().suffix(".go").tempfile().unwrap();
    write!(f, r#"resp, err := http.Post("https://api.example.com/orders", "application/json", body)"#).unwrap();

    let root = f.path().parent().unwrap();
    let file_name = f.path().file_name().unwrap().to_str().unwrap();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'go', 0)",
        [file_name],
    )
    .unwrap();

    let calls = detect_http_calls_all_languages(conn, root).unwrap();
    let go_calls: Vec<_> = calls
        .iter()
        .filter(|c| c.raw_url.contains("api.example.com"))
        .collect();
    assert_eq!(go_calls.len(), 1);
    assert_eq!(go_calls[0].http_method, "POST");
}

#[test]
fn detect_csharp_httpclient_get_async() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let mut f = tempfile::Builder::new().suffix(".cs").tempfile().unwrap();
    write!(
        f,
        r#"var response = await HttpClient.GetAsync("https://api.example.com/catalog");"#
    )
    .unwrap();

    let root = f.path().parent().unwrap();
    let file_name = f.path().file_name().unwrap().to_str().unwrap();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'csharp', 0)",
        [file_name],
    )
    .unwrap();

    let calls = detect_http_calls_all_languages(conn, root).unwrap();
    let cs_calls: Vec<_> = calls
        .iter()
        .filter(|c| c.raw_url.contains("api.example.com"))
        .collect();
    assert_eq!(cs_calls.len(), 1);
    assert_eq!(cs_calls[0].http_method, "GET");
}

#[test]
fn normalise_method_maps_java_convenience_methods() {
    assert_eq!(normalise_method("getForObject"), "GET");
    assert_eq!(normalise_method("postForEntity"), "POST");
    assert_eq!(normalise_method("DELETE"), "DELETE");
}

#[test]
fn detect_ruby_httparty_post() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let mut f = tempfile::Builder::new().suffix(".rb").tempfile().unwrap();
    write!(f, r#"response = HTTParty.post("https://api.example.com/submit", body: data)"#)
        .unwrap();

    let root = f.path().parent().unwrap();
    let file_name = f.path().file_name().unwrap().to_str().unwrap();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed) VALUES (?1, 'h', 'ruby', 0)",
        [file_name],
    )
    .unwrap();

    let calls = detect_http_calls_all_languages(conn, root).unwrap();
    let ruby_calls: Vec<_> = calls
        .iter()
        .filter(|c| c.raw_url.contains("api.example.com"))
        .collect();
    assert_eq!(ruby_calls.len(), 1);
    assert_eq!(ruby_calls[0].http_method, "POST");
}
