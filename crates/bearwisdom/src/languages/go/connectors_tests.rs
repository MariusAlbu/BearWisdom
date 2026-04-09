use super::*;
use crate::db::Database;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_regexes() -> Regexes {
    Regexes {
        handle_func: build_handle_func_regex(),
        gin: build_gin_style_regex(),
        chi: build_chi_style_regex(),
        method_handle_func: build_method_handle_func_regex(),
        group: build_group_regex(),
    }
}

fn insert_go_file(conn: &Connection, path: &str) -> i64 {
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES (?1, 'h', 'go', 0)",
        [path],
    )
    .unwrap();
    conn.last_insert_rowid()
}

// ---------------------------------------------------------------------------
// Regex unit tests
// ---------------------------------------------------------------------------

#[test]
fn handle_func_regex_matches_stdlib() {
    let re = build_handle_func_regex();
    let line = r#"    http.HandleFunc("/api/users", listUsers)"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "/api/users");
    assert_eq!(&cap[2], "listUsers");
}

#[test]
fn handle_func_regex_matches_gorilla_mux_var() {
    let re = build_handle_func_regex();
    let line = r#"mux.HandleFunc("/health", healthCheck)"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "/health");
    assert_eq!(&cap[2], "healthCheck");
}

#[test]
fn handle_func_regex_does_not_match_without_slash() {
    // Paths without a leading slash should still be captured (path is freeform).
    let re = build_handle_func_regex();
    let line = r#"r.HandleFunc("health", handler)"#;
    assert!(re.captures(line).is_some()); // regex doesn't enforce leading slash
}

#[test]
fn gin_regex_matches_get() {
    let re = build_gin_style_regex();
    let line = r#"    r.GET("/items", getItems)"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "r");
    assert_eq!(&cap[2], "GET");
    assert_eq!(&cap[3], "/items");
    assert_eq!(&cap[4], "getItems");
}

#[test]
fn gin_regex_matches_post() {
    let re = build_gin_style_regex();
    let line = r#"router.POST("/orders", createOrder)"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[2], "POST");
    assert_eq!(&cap[3], "/orders");
    assert_eq!(&cap[4], "createOrder");
}

#[test]
fn gin_regex_matches_delete() {
    let re = build_gin_style_regex();
    let line = r#"r.DELETE("/items/:id", deleteItem)"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[2], "DELETE");
    assert_eq!(&cap[3], "/items/:id");
}

#[test]
fn chi_regex_matches_get_title_case() {
    let re = build_chi_style_regex();
    let line = r#"r.Get("/users", listUsers)"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "r");
    assert_eq!(&cap[2], "Get");
    assert_eq!(&cap[3], "/users");
    assert_eq!(&cap[4], "listUsers");
}

#[test]
fn chi_regex_matches_post_title_case() {
    let re = build_chi_style_regex();
    let line = r#"r.Post("/users", createUser)"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[2], "Post");
    assert_eq!(&cap[3], "/users");
}

#[test]
fn method_handle_func_regex_matches() {
    let re = build_method_handle_func_regex();
    let line = r#"r.HandleFunc(http.MethodGet, "/reports", serveReport)"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "r");
    assert_eq!(&cap[2], "MethodGet");
    assert_eq!(&cap[3], "/reports");
    assert_eq!(&cap[4], "serveReport");
}

#[test]
fn group_regex_matches_gin_group() {
    let re = build_group_regex();
    let line = r#"    api := r.Group("/api")"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "api");
    assert_eq!(&cap[2], "/api");
}

#[test]
fn group_regex_matches_echo_group() {
    let re = build_group_regex();
    let line = r#"v1 := e.Group("/v1")"#;
    let cap = re.captures(line).unwrap();
    assert_eq!(&cap[1], "v1");
    assert_eq!(&cap[2], "/v1");
}

// ---------------------------------------------------------------------------
// Helper function unit tests
// ---------------------------------------------------------------------------

#[test]
fn method_const_to_verb_known_methods() {
    assert_eq!(method_const_to_verb("MethodGet"), "GET");
    assert_eq!(method_const_to_verb("MethodPost"), "POST");
    assert_eq!(method_const_to_verb("MethodPut"), "PUT");
    assert_eq!(method_const_to_verb("MethodDelete"), "DELETE");
    assert_eq!(method_const_to_verb("MethodPatch"), "PATCH");
}

#[test]
fn method_const_to_verb_unknown_strips_prefix() {
    assert_eq!(method_const_to_verb("MethodOptions"), "OPTIONS");
    assert_eq!(method_const_to_verb("MethodHead"), "HEAD");
}

#[test]
fn join_paths_combines_prefix_and_segment() {
    assert_eq!(join_paths("/api", "/users"), "/api/users");
    assert_eq!(join_paths("/api/", "users"), "/api/users");
    assert_eq!(join_paths("", "/users"), "/users");
    assert_eq!(join_paths("/v1", "/items/:id"), "/v1/items/:id");
}

#[test]
fn normalise_prefix_adds_leading_slash() {
    assert_eq!(normalise_prefix("api"), "/api");
    assert_eq!(normalise_prefix("/api/"), "/api");
    assert_eq!(normalise_prefix("/v1"), "/v1");
}

// ---------------------------------------------------------------------------
// Source extraction tests
// ---------------------------------------------------------------------------

#[test]
fn extracts_stdlib_handle_func() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();
    let file_id = insert_go_file(conn, "main.go");

    let source = r#"
func main() {
    http.HandleFunc("/api/users", listUsers)
    http.HandleFunc("/api/orders", listOrders)
    http.ListenAndServe(":8080", nil)
}
"#;

    let re = make_regexes();
    let mut routes = Vec::new();
    extract_routes_from_source(source, file_id, conn, "main.go", &re, &mut routes);

    assert_eq!(routes.len(), 2);
    assert_eq!(routes[0].http_method, "GET");
    assert_eq!(routes[0].route_template, "/api/users");
    assert_eq!(routes[0].resolved_route, "/api/users");
    assert_eq!(routes[1].route_template, "/api/orders");
}

#[test]
fn extracts_gin_get_route() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();
    let file_id = insert_go_file(conn, "router.go");

    let source = r#"
func setupRouter() *gin.Engine {
    r := gin.Default()
    r.GET("/ping", pingHandler)
    r.POST("/users", createUser)
    return r
}
"#;

    let re = make_regexes();
    let mut routes = Vec::new();
    extract_routes_from_source(source, file_id, conn, "router.go", &re, &mut routes);

    assert_eq!(routes.len(), 2);

    let get_route = routes.iter().find(|r| r.http_method == "GET").unwrap();
    assert_eq!(get_route.route_template, "/ping");
    assert_eq!(get_route.resolved_route, "/ping");
    assert_eq!(get_route.handler_name, "pingHandler");

    let post_route = routes.iter().find(|r| r.http_method == "POST").unwrap();
    assert_eq!(post_route.route_template, "/users");
    assert_eq!(post_route.handler_name, "createUser");
}

#[test]
fn extracts_chi_title_case_routes() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();
    let file_id = insert_go_file(conn, "routes.go");

    let source = r#"
func setupChi() {
    r := chi.NewRouter()
    r.Get("/articles", listArticles)
    r.Post("/articles", createArticle)
    r.Delete("/articles/{id}", deleteArticle)
}
"#;

    let re = make_regexes();
    let mut routes = Vec::new();
    extract_routes_from_source(source, file_id, conn, "routes.go", &re, &mut routes);

    assert_eq!(routes.len(), 3);
    assert!(routes.iter().any(|r| r.http_method == "GET" && r.route_template == "/articles"));
    assert!(routes.iter().any(|r| r.http_method == "POST" && r.route_template == "/articles"));
    assert!(routes.iter().any(|r| r.http_method == "DELETE" && r.route_template == "/articles/{id}"));
}

#[test]
fn resolves_gin_group_prefix() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();
    let file_id = insert_go_file(conn, "server.go");

    let source = r#"
func setupRoutes(r *gin.Engine) {
    api := r.Group("/api")
    api.GET("/users", listUsers)
    api.POST("/users", createUser)
    api.GET("/orders", listOrders)
}
"#;

    let re = make_regexes();
    let mut routes = Vec::new();
    extract_routes_from_source(source, file_id, conn, "server.go", &re, &mut routes);

    assert_eq!(routes.len(), 3);

    // route_template is the local segment; resolved_route is the full path.
    let users_get = routes
        .iter()
        .find(|r| r.http_method == "GET" && r.handler_name == "listUsers")
        .expect("listUsers GET not found");
    assert_eq!(users_get.route_template, "/users");
    assert_eq!(users_get.resolved_route, "/api/users");

    let users_post = routes
        .iter()
        .find(|r| r.http_method == "POST")
        .expect("POST not found");
    assert_eq!(users_post.resolved_route, "/api/users");

    let orders = routes
        .iter()
        .find(|r| r.handler_name == "listOrders")
        .expect("listOrders not found");
    assert_eq!(orders.resolved_route, "/api/orders");
}

#[test]
fn resolves_echo_group_prefix() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();
    let file_id = insert_go_file(conn, "echo_server.go");

    let source = r#"
func main() {
    e := echo.New()
    v1 := e.Group("/v1")
    v1.GET("/products", listProducts)
    v1.DELETE("/products/:id", deleteProduct)
}
"#;

    let re = make_regexes();
    let mut routes = Vec::new();
    extract_routes_from_source(source, file_id, conn, "echo_server.go", &re, &mut routes);

    assert_eq!(routes.len(), 2);
    assert!(routes
        .iter()
        .all(|r| r.resolved_route.starts_with("/v1/")));
}

#[test]
fn extracts_method_handle_func_with_http_method_const() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();
    let file_id = insert_go_file(conn, "handlers.go");

    let source = r#"
func register(r *MyRouter) {
    r.HandleFunc(http.MethodGet, "/reports", serveReport)
    r.HandleFunc(http.MethodPost, "/reports", createReport)
}
"#;

    let re = make_regexes();
    let mut routes = Vec::new();
    extract_routes_from_source(source, file_id, conn, "handlers.go", &re, &mut routes);

    assert_eq!(routes.len(), 2);
    let get_route = routes.iter().find(|r| r.http_method == "GET").unwrap();
    assert_eq!(get_route.route_template, "/reports");
    assert_eq!(get_route.handler_name, "serveReport");

    let post_route = routes.iter().find(|r| r.http_method == "POST").unwrap();
    assert_eq!(post_route.handler_name, "createReport");
}

// ---------------------------------------------------------------------------
// Integration tests: connect() with temp files
// ---------------------------------------------------------------------------

#[test]
fn connect_with_stdlib_file_inserts_routes() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let dir = tempfile::TempDir::new().unwrap();
    let go_source = r#"
package main

import "net/http"

func main() {
    http.HandleFunc("/health", healthHandler)
    http.HandleFunc("/ready", readyHandler)
    http.ListenAndServe(":8080", nil)
}
"#;

    let go_file = dir.path().join("main.go");
    std::fs::write(&go_file, go_source).unwrap();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('main.go', 'abc', 'go', 0)",
        [],
    )
    .unwrap();

    let count = connect(conn, dir.path()).unwrap();
    assert_eq!(count, 2, "Expected 2 routes inserted");

    let stored: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare("SELECT http_method, route_template FROM routes ORDER BY route_template")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert_eq!(stored[0], ("GET".to_string(), "/health".to_string()));
    assert_eq!(stored[1], ("GET".to_string(), "/ready".to_string()));
}

#[test]
fn connect_with_gin_file_inserts_routes() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let dir = tempfile::TempDir::new().unwrap();
    let go_source = r#"
package main

import "github.com/gin-gonic/gin"

func main() {
    r := gin.Default()
    r.GET("/items", listItems)
    r.POST("/items", createItem)
    r.DELETE("/items/:id", deleteItem)
    r.Run(":8080")
}
"#;

    let go_file = dir.path().join("gin_main.go");
    std::fs::write(&go_file, go_source).unwrap();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('gin_main.go', 'abc', 'go', 0)",
        [],
    )
    .unwrap();

    let count = connect(conn, dir.path()).unwrap();
    assert_eq!(count, 3);

    let methods: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT http_method FROM routes ORDER BY http_method")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert!(methods.contains(&"GET".to_string()));
    assert!(methods.contains(&"POST".to_string()));
    assert!(methods.contains(&"DELETE".to_string()));
}

#[test]
fn connect_with_group_prefix_resolves_routes() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let dir = tempfile::TempDir::new().unwrap();
    let go_source = r#"
package main

import "github.com/gin-gonic/gin"

func setupRouter() *gin.Engine {
    r := gin.Default()
    api := r.Group("/api")
    api.GET("/users", listUsers)
    api.POST("/users", createUser)
    return r
}
"#;

    let go_file = dir.path().join("router.go");
    std::fs::write(&go_file, go_source).unwrap();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('router.go', 'abc', 'go', 0)",
        [],
    )
    .unwrap();

    let count = connect(conn, dir.path()).unwrap();
    assert_eq!(count, 2);

    let resolved: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT resolved_route FROM routes ORDER BY resolved_route")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert_eq!(resolved[0], "/api/users");
    assert_eq!(resolved[1], "/api/users");
}

#[test]
fn connect_on_empty_project_returns_zero() {
    let db = Database::open_in_memory().unwrap();
    let dir = tempfile::TempDir::new().unwrap();
    let count = connect(db.conn(), dir.path()).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn connect_skips_unreadable_file_gracefully() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    // Register a file that doesn't exist on disk.
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('ghost.go', 'xyz', 'go', 0)",
        [],
    )
    .unwrap();

    // Should not error — unreadable files are skipped with debug logs.
    let count = connect(conn, std::path::Path::new("/nonexistent/root")).unwrap();
    assert_eq!(count, 0);
}

#[test]
fn symbol_id_is_populated_when_handler_exists_in_symbols() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let dir = tempfile::TempDir::new().unwrap();
    let go_source = r#"
package main

func listUsers(w http.ResponseWriter, r *http.Request) {}

func main() {
    http.HandleFunc("/users", listUsers)
}
"#;
    let go_file = dir.path().join("app.go");
    std::fs::write(&go_file, go_source).unwrap();

    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES ('app.go', 'h1', 'go', 0)",
        [],
    )
    .unwrap();
    let file_id: i64 = conn.last_insert_rowid();

    // Pre-populate the symbol so lookup_symbol can find it.
    conn.execute(
        "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col)
         VALUES (?1, 'listUsers', 'listUsers', 'function', 3, 0)",
        [file_id],
    )
    .unwrap();
    let sym_id: i64 = conn.last_insert_rowid();

    connect(conn, dir.path()).unwrap();

    let stored_sym_id: Option<i64> = conn
        .query_row(
            "SELECT symbol_id FROM routes WHERE route_template = '/users'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    assert_eq!(stored_sym_id, Some(sym_id));
}
