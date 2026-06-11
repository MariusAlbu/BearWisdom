use super::extract as run_extract;
use crate::languages::erlang::cowboy::extract_cowboy_triples_from_text;
use crate::types::{EdgeKind, ExtractedRoute};

/// Collect the `target_name`s of all `Calls` refs from a full extract.
fn call_targets(src: &str) -> Vec<String> {
    run_extract(src)
        .refs
        .into_iter()
        .filter(|r| r.kind == EdgeKind::Calls)
        .map(|r| r.target_name)
        .collect()
}

#[test]
fn higher_order_variable_call_not_emitted() {
    // `Fun(X)` invokes a bound variable (capitalized = variable in Erlang).
    // It names no function symbol and must not produce a Calls ref.
    let src = r#"
-module(myapp).
-export([run/2]).

run(Fun, X) ->
    Fun(X).
"#;
    let targets = call_targets(src);
    assert!(
        !targets.contains(&"Fun".to_string()) && !targets.contains(&"Fun/1".to_string()),
        "variable callee `Fun(X)` must not emit a Calls ref; got {targets:?}"
    );
}

#[test]
fn higher_order_remote_variable_fun_not_emitted() {
    // `Mod:Fun(X)` — the function part is a variable; suppress like the bare form.
    let src = r#"
-module(myapp).
-export([run/3]).

run(Mod, Fun, X) ->
    Mod:Fun(X).
"#;
    let targets = call_targets(src);
    assert!(
        !targets.contains(&"Fun".to_string()) && !targets.contains(&"Fun/1".to_string()),
        "variable remote-fun `Mod:Fun(X)` must not emit a Calls ref; got {targets:?}"
    );
}

#[test]
fn named_call_still_emitted() {
    // Guard: an ordinary named call must still emit an arity-qualified Calls ref.
    let src = r#"
-module(myapp).
-export([run/1]).

run(X) ->
    helper(X).

helper(Y) -> Y.
"#;
    let targets = call_targets(src);
    assert!(
        targets.contains(&"helper/1".to_string()),
        "named call `helper(X)` must still emit; got {targets:?}"
    );
}

#[test]
fn named_remote_call_still_emitted() {
    // Guard: a named remote call (`lists:reverse(X)`) must still emit.
    let src = r#"
-module(myapp).
-export([run/1]).

run(X) ->
    lists:reverse(X).
"#;
    let targets = call_targets(src);
    assert!(
        targets.contains(&"reverse/1".to_string()),
        "named remote call must still emit; got {targets:?}"
    );
}

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
