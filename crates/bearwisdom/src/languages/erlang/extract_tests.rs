use super::extract_cowboy_triples_from_text;
use crate::types::ExtractedRoute;
use super::extract as run_extract;

fn collect(text: &str) -> Vec<ExtractedRoute> {
    let mut routes = Vec::new();
    extract_cowboy_triples_from_text(text, &mut routes, &[], 1);
    routes
}

#[test]
fn cowboy_simple_route_triple() {
    let text = r#"[{"/users", users_handler, []}]"#;
    let routes = collect(text);
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].template, "/users");
    assert_eq!(routes[0].http_method, "");
}

#[test]
fn cowboy_dispatch_with_host_wrapper() {
    let text = r#"[
        {'_', [
            {"/users", users_handler, []},
            {"/users/:id", user_handler, []},
            {"/health", health_handler, []}
        ]}
    ]"#;
    let routes = collect(text);
    assert_eq!(routes.len(), 3);
    let templates: Vec<_> = routes.iter().map(|r| r.template.as_str()).collect();
    assert!(templates.contains(&"/users"));
    assert!(templates.contains(&"/users/{id}"));
    assert!(templates.contains(&"/health"));
}

#[test]
fn cowboy_bind_segment_normalized() {
    let text = r#"[{"/users/:user_id/posts/:post_id", post_handler, []}]"#;
    let routes = collect(text);
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].template, "/users/{user_id}/posts/{post_id}");
}

#[test]
fn cowboy_optional_segments_stripped() {
    let text = r#"[{"/api[/:version]/users", users_handler, []}]"#;
    let routes = collect(text);
    assert_eq!(routes.len(), 1);
    assert_eq!(routes[0].template, "/api/{version}/users");
}

#[test]
fn cowboy_non_route_tuple_skipped() {
    // First element must be a slash-prefixed string. Tuples whose first
    // element is an atom or any non-route shape are skipped.
    let text = r#"{'_', [{ok, ignored}]}"#;
    let routes = collect(text);
    assert!(routes.is_empty());
}

#[test]
fn cowboy_via_full_extract() {
    let src = r#"
-module(myapp).
-export([init/2]).

init(_Req, State) ->
    Dispatch = cowboy_router:compile([
        {'_', [
            {"/api/users", users_handler, []},
            {"/api/users/:id", user_handler, []}
        ]}
    ]),
    {ok, Dispatch, State}.
"#;
    let r = run_extract(src);
    let templates: Vec<_> = r.routes.iter().map(|x| x.template.as_str()).collect();
    assert!(templates.contains(&"/api/users"));
    assert!(templates.contains(&"/api/users/{id}"));
}

#[test]
fn cowboy_no_emit_for_non_cowboy_calls() {
    let src = r#"
-module(myapp).
-export([f/0]).

f() ->
    lists:foreach(fun (X) -> io:format("~p", [X]) end, [{"/users", h, []}]).
"#;
    let r = run_extract(src);
    // The tuple is in a generic list passed to lists:foreach, not to
    // cowboy_router:compile — must not emit.
    assert!(r.routes.is_empty());
}
