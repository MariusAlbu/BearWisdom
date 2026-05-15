use super::extract_servant_routes;
use crate::types::ExtractedRoute;

fn collect(src: &str) -> Vec<ExtractedRoute> {
    let mut routes = Vec::new();
    extract_servant_routes(src, 0, &mut routes);
    routes
}

#[test]
fn servant_simple_get_route() {
    let routes = collect(r#"type API = "users" :> Get '[JSON] [User]"#);
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].http_method, "GET");
    assert_eq!(routes[0].template, "/users");
}

#[test]
fn servant_capture_yields_parametric_segment() {
    let routes = collect(
        r#"type API = "users" :> Capture "id" Int :> Get '[JSON] User"#,
    );
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].template, "/users/{id}");
    assert_eq!(routes[0].http_method, "GET");
}

#[test]
fn servant_alternative_split_yields_multiple_routes() {
    let routes = collect(
        r#"type API =
                  "users" :> Get '[JSON] [User]
             :<|> "users" :> Capture "id" Int :> Get '[JSON] User
             :<|> "users" :> ReqBody '[JSON] User :> Post '[JSON] User
             :<|> "users" :> Capture "id" Int :> Delete '[JSON] NoContent"#,
    );
    assert_eq!(routes.len(), 4);
    assert_eq!(routes[0].template, "/users");
    assert_eq!(routes[0].http_method, "GET");
    assert_eq!(routes[1].template, "/users/{id}");
    assert_eq!(routes[1].http_method, "GET");
    assert_eq!(routes[2].template, "/users");
    assert_eq!(routes[2].http_method, "POST");
    assert_eq!(routes[3].template, "/users/{id}");
    assert_eq!(routes[3].http_method, "DELETE");
}

#[test]
fn servant_reqbody_and_querystring_metadata_ignored_in_path() {
    let routes = collect(
        r#"type API = "search" :> QueryParam "q" Text :> Header "X-Auth" Text :> Get '[JSON] [Item]"#,
    );
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].template, "/search");
}

#[test]
fn servant_nested_path_segments() {
    let routes = collect(
        r#"type API = "api" :> "v1" :> "users" :> Capture "id" Int :> Get '[JSON] User"#,
    );
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].template, "/api/v1/users/{id}");
}

#[test]
fn servant_explicit_verb_form_recognised() {
    let routes = collect(
        r#"type API = "ping" :> Verb 'GET 200 '[JSON] Status"#,
    );
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].http_method, "GET");
    assert_eq!(routes[0].template, "/ping");
}

#[test]
fn servant_non_servant_type_synonym_emits_nothing() {
    let routes = collect("type UserId = Int");
    assert!(routes.is_empty());
}

#[test]
fn servant_handler_index_propagated() {
    let mut routes = Vec::new();
    extract_servant_routes(
        r#"type API = "x" :> Get '[JSON] Int"#,
        42,
        &mut routes,
    );
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].handler_symbol_index, 42);
}
