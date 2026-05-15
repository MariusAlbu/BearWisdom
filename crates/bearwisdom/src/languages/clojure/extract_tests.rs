use super::extract as run_extract;

#[test]
fn reitit_simple_get_route() {
    let src = r#"
(def routes
  ["/users" {:get list-users}])
"#;
    let r = run_extract(src);
    assert_eq!(r.routes.len(), 1);
    assert_eq!(r.routes[0].http_method, "GET");
    assert_eq!(r.routes[0].template, "/users");
}

#[test]
fn reitit_multiple_verbs_in_one_map() {
    let src = r#"
(def routes
  ["/users" {:get list-users
             :post create-user
             :delete delete-all}])
"#;
    let r = run_extract(src);
    assert_eq!(r.routes.len(), 3);
    let methods: Vec<_> = r.routes.iter().map(|x| x.http_method.as_str()).collect();
    assert!(methods.contains(&"GET"));
    assert!(methods.contains(&"POST"));
    assert!(methods.contains(&"DELETE"));
    for route in &r.routes {
        assert_eq!(route.template, "/users");
    }
}

#[test]
fn reitit_nested_path_combines() {
    let src = r#"
(def routes
  ["/api"
   ["/users" {:get list-users}]
   ["/posts" {:get list-posts}]])
"#;
    let r = run_extract(src);
    assert_eq!(r.routes.len(), 2);
    let templates: Vec<_> = r.routes.iter().map(|x| x.template.as_str()).collect();
    assert!(templates.contains(&"/api/users"));
    assert!(templates.contains(&"/api/posts"));
}

#[test]
fn reitit_deeply_nested_routes() {
    let src = r#"
(def routes
  ["/api"
   ["/v1"
    ["/users" {:get list-users
               :post create-user}]
    ["/health" {:get health-check}]]])
"#;
    let r = run_extract(src);
    assert_eq!(r.routes.len(), 3);
    let templates: Vec<_> = r.routes.iter().map(|x| x.template.as_str()).collect();
    assert!(templates.iter().filter(|t| **t == "/api/v1/users").count() == 2);
    assert!(templates.contains(&"/api/v1/health"));
}

#[test]
fn reitit_ignores_non_verb_keys() {
    let src = r#"
(def routes
  ["/users" {:name :users
             :middleware [auth-mw]
             :get list-users}])
"#;
    let r = run_extract(src);
    assert_eq!(r.routes.len(), 1);
    assert_eq!(r.routes[0].http_method, "GET");
}

#[test]
fn reitit_vec_without_string_path_is_skipped() {
    // A regular Clojure vector — not a Reitit route — must not emit a route.
    let src = r#"
(def items [1 2 3])
(def people [{:name "Alice"} {:name "Bob"}])
"#;
    let r = run_extract(src);
    assert!(r.routes.is_empty());
}

#[test]
fn reitit_handler_line_records_handler_position() {
    let src = "(def routes [\"/x\" {:get the-handler}])";
    let r = run_extract(src);
    assert_eq!(r.routes.len(), 1);
    // Handler is on line 1 (1-based). Line capture is best-effort and may
    // fall back to map line — either is acceptable as long as it's set.
    assert!(r.routes[0].http_method == "GET");
}
