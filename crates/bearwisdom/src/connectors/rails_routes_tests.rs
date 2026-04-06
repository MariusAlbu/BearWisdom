use super::*;
use crate::db::Database;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn insert_ruby_file(conn: &Connection, path: &str) -> i64 {
    conn.execute(
        "INSERT INTO files (path, hash, language, last_indexed)
         VALUES (?1, 'h', 'ruby', 0)",
        [path],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn route_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM routes", [], |r| r.get(0))
        .unwrap()
}

fn routes_for_method(conn: &Connection, method: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT route_template FROM routes WHERE http_method = ?1 ORDER BY route_template",
        )
        .unwrap();
    stmt.query_map([method], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

// ---------------------------------------------------------------------------
// Unit tests — regex / parser (no DB)
// ---------------------------------------------------------------------------

#[test]
fn verb_regex_matches_get_with_single_quotes() {
    let re = build_verb_regex();
    let cap = re.captures("  get '/users', to: 'users#index'").unwrap();
    assert_eq!(&cap[1], "get");
    assert_eq!(&cap[2], "/users");
}

#[test]
fn verb_regex_matches_post_with_double_quotes() {
    let re = build_verb_regex();
    let cap = re.captures(r#"  post "/orders", to: "orders#create""#).unwrap();
    assert_eq!(&cap[1], "post");
    assert_eq!(&cap[2], "/orders");
}

#[test]
fn verb_regex_does_not_match_plain_ruby_method_call() {
    let re = build_verb_regex();
    assert!(!re.is_match("  puts 'hello'"));
    assert!(!re.is_match("  render json: { status: 'ok' }"));
}

#[test]
fn resources_regex_matches_bare_resources() {
    let re = build_resources_regex();
    let cap = re.captures("  resources :articles").unwrap();
    assert_eq!(&cap[1], "articles");
    assert!(cap.get(2).is_none());
}

#[test]
fn resources_regex_matches_resources_with_only_filter() {
    let re = build_resources_regex();
    let cap = re
        .captures("  resources :comments, only: [:index, :show]")
        .unwrap();
    assert_eq!(&cap[1], "comments");
    assert_eq!(cap[2].trim(), ":index, :show");
}

#[test]
fn namespace_regex_matches_namespace_do() {
    let re = build_namespace_regex();
    let cap = re.captures("  namespace :api do").unwrap();
    assert_eq!(&cap[1], "api");
}

#[test]
fn scope_regex_matches_scope_path_do() {
    let re = build_scope_regex();
    let cap = re.captures(r#"  scope '/v1' do"#).unwrap();
    assert_eq!(&cap[1], "/v1");
}

#[test]
fn normalise_method_uppercases_verbs() {
    assert_eq!(normalise_method("get"), "GET");
    assert_eq!(normalise_method("POST"), "POST");
    assert_eq!(normalise_method("match"), "GET");
}

// ---------------------------------------------------------------------------
// Test 1 — explicit verb routes
// ---------------------------------------------------------------------------

#[test]
fn parse_explicit_verb_routes() {
    let source = r#"
Rails.application.routes.draw do
  get  '/users',     to: 'users#index'
  post '/users',     to: 'users#create'
  put  '/users/:id', to: 'users#update'
  delete '/users/:id', to: 'users#destroy'
end
"#;
    let entries = parse_routes_source(source);

    assert_eq!(entries.len(), 4, "expected 4 explicit routes");

    let methods: Vec<&str> = entries.iter().map(|e| e.http_method).collect();
    assert!(methods.contains(&"GET"));
    assert!(methods.contains(&"POST"));
    assert!(methods.contains(&"PUT"));
    assert!(methods.contains(&"DELETE"));

    let templates: Vec<&str> = entries.iter().map(|e| e.route_template.as_str()).collect();
    assert!(templates.contains(&"/users"));
    assert!(templates.contains(&"/users/:id"));
}

// ---------------------------------------------------------------------------
// Test 2 — resources expansion
// ---------------------------------------------------------------------------

#[test]
fn parse_resources_expands_to_seven_routes() {
    let source = r#"
Rails.application.routes.draw do
  resources :articles
end
"#;
    let entries = parse_routes_source(source);

    assert_eq!(entries.len(), 7, "resources :articles should yield 7 routes");

    let templates: Vec<&str> = entries.iter().map(|e| e.route_template.as_str()).collect();
    assert!(templates.contains(&"/articles"), "index route missing");
    assert!(templates.contains(&"/articles/:id"), "show route missing");
    assert!(templates.contains(&"/articles/new"), "new route missing");
    assert!(templates.contains(&"/articles/:id/edit"), "edit route missing");
}

#[test]
fn parse_resources_with_only_filter() {
    let source = r#"
Rails.application.routes.draw do
  resources :comments, only: [:index, :show]
end
"#;
    let entries = parse_routes_source(source);

    assert_eq!(entries.len(), 2, "only: [:index, :show] should yield 2 routes");

    let templates: Vec<&str> = entries.iter().map(|e| e.route_template.as_str()).collect();
    assert!(templates.contains(&"/comments"));
    assert!(templates.contains(&"/comments/:id"));
}

// ---------------------------------------------------------------------------
// Test 3 — namespaced routes
// ---------------------------------------------------------------------------

#[test]
fn parse_namespaced_routes_apply_prefix() {
    let source = r#"
Rails.application.routes.draw do
  namespace :api do
    get '/status', to: 'api/status#index'
    resources :users
  end
end
"#;
    let entries = parse_routes_source(source);

    // /api/status + 7 resources routes under /api
    assert!(
        entries.len() >= 8,
        "expected at least 8 routes inside namespace :api, got {}",
        entries.len()
    );

    for entry in &entries {
        assert!(
            entry.route_template.starts_with("/api"),
            "route '{}' should be prefixed with /api",
            entry.route_template
        );
    }
}

#[test]
fn parse_scope_routes_apply_prefix() {
    let source = r#"
Rails.application.routes.draw do
  scope '/v2' do
    get '/ping', to: 'ping#index'
  end
end
"#;
    let entries = parse_routes_source(source);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].route_template, "/v2/ping");
}

// ---------------------------------------------------------------------------
// Test 4 — root declaration
// ---------------------------------------------------------------------------

#[test]
fn parse_root_emits_get_slash() {
    let source = r#"
Rails.application.routes.draw do
  root 'home#index'
end
"#;
    let entries = parse_routes_source(source);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].http_method, "GET");
    assert!(entries[0].route_template.ends_with('/'));
}

// ---------------------------------------------------------------------------
// Integration tests — DB insertion
// ---------------------------------------------------------------------------

#[test]
fn connect_inserts_explicit_routes_into_db() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let dir = tempfile::TempDir::new().unwrap();
    let routes_dir = dir.path().join("config");
    std::fs::create_dir_all(&routes_dir).unwrap();
    std::fs::write(
        routes_dir.join("routes.rb"),
        "Rails.application.routes.draw do\n  get '/users', to: 'users#index'\n  post '/users', to: 'users#create'\nend\n",
    )
    .unwrap();

    insert_ruby_file(conn, "config/routes.rb");

    let count = connect(conn, dir.path()).unwrap();
    assert_eq!(count, 2, "expected 2 routes inserted");
    assert_eq!(route_count(conn), 2);

    let gets = routes_for_method(conn, "GET");
    assert_eq!(gets, vec!["/users"]);
}

#[test]
fn connect_expands_resources_in_db() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let dir = tempfile::TempDir::new().unwrap();
    let routes_dir = dir.path().join("config");
    std::fs::create_dir_all(&routes_dir).unwrap();
    std::fs::write(
        routes_dir.join("routes.rb"),
        "Rails.application.routes.draw do\n  resources :posts\nend\n",
    )
    .unwrap();

    insert_ruby_file(conn, "config/routes.rb");

    let count = connect(conn, dir.path()).unwrap();
    assert_eq!(count, 7, "resources :posts should insert 7 rows");
}

#[test]
fn connect_inserts_namespaced_routes_in_db() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();

    let dir = tempfile::TempDir::new().unwrap();
    let routes_dir = dir.path().join("config");
    std::fs::create_dir_all(&routes_dir).unwrap();
    std::fs::write(
        routes_dir.join("routes.rb"),
        "Rails.application.routes.draw do\n  namespace :api do\n    get '/ping', to: 'api/ping#index'\n  end\nend\n",
    )
    .unwrap();

    insert_ruby_file(conn, "config/routes.rb");

    let count = connect(conn, dir.path()).unwrap();
    assert_eq!(count, 1);

    let gets = routes_for_method(conn, "GET");
    assert_eq!(gets, vec!["/api/ping"]);
}

#[test]
fn connect_with_empty_project_returns_zero() {
    let db = Database::open_in_memory().unwrap();
    let dir = tempfile::TempDir::new().unwrap();
    let count = connect(db.conn(), dir.path()).unwrap();
    assert_eq!(count, 0);
}
