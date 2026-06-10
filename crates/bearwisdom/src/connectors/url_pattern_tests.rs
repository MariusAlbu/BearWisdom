// Tests for connectors/url_pattern.rs

use super::*;

// ---------------------------------------------------------------------------
// normalize — path-parameter syntax variants
// ---------------------------------------------------------------------------

#[test]
fn normalize_express_colon_id() {
    assert_eq!(normalize("/api/users/:id"), "/api/users/{}");
}

#[test]
fn normalize_django_angle_bracket() {
    assert_eq!(normalize("/api/users/<id>"), "/api/users/{}");
}

#[test]
fn normalize_django_typed_angle_bracket() {
    assert_eq!(normalize("/api/users/<int:pk>"), "/api/users/{}");
}

#[test]
fn normalize_fastapi_brace_id() {
    assert_eq!(normalize("/api/users/{id}"), "/api/users/{}");
}

#[test]
fn normalize_already_canonical() {
    assert_eq!(normalize("/api/users/{}"), "/api/users/{}");
}

#[test]
fn normalize_multiple_params() {
    assert_eq!(
        normalize("/api/users/:id/posts/:postId"),
        "/api/users/{}/posts/{}"
    );
}

#[test]
fn normalize_multiple_params_mixed_syntax() {
    assert_eq!(
        normalize("/api/users/{userId}/posts/:postId"),
        "/api/users/{}/posts/{}"
    );
}

#[test]
fn normalize_no_params_unchanged() {
    assert_eq!(normalize("/api/users"), "/api/users");
}

#[test]
fn normalize_root_path() {
    assert_eq!(normalize("/"), "/");
}

#[test]
fn normalize_empty_string() {
    assert_eq!(normalize(""), "/");
}

#[test]
fn normalize_trailing_slash_removed() {
    assert_eq!(normalize("/api/users/"), "/api/users");
}

#[test]
fn normalize_root_trailing_slash_kept() {
    // Root "/" should not be collapsed to "".
    assert_eq!(normalize("/"), "/");
}

#[test]
fn normalize_query_string_stripped() {
    assert_eq!(normalize("/api/users?page=1&limit=20"), "/api/users");
}

#[test]
fn normalize_query_string_with_param_stripped() {
    assert_eq!(normalize("/api/users/:id?include=posts"), "/api/users/{}");
}

#[test]
fn normalize_spring_path_variable() {
    assert_eq!(
        normalize("/api/items/{itemId}/reviews/{reviewId}"),
        "/api/items/{}/reviews/{}"
    );
}

#[test]
fn normalize_no_leading_slash() {
    // Bare path without leading slash — normalizer handles gracefully.
    assert_eq!(normalize("api/users/:id"), "/api/users/{}");
}

// ---------------------------------------------------------------------------
// http_methods_compatible
// ---------------------------------------------------------------------------

#[test]
fn http_compat_any_wildcard_matches_get() {
    assert!(http_methods_compatible(Some("*"), Some("GET")));
}

#[test]
fn http_compat_get_matches_any_wildcard() {
    assert!(http_methods_compatible(Some("GET"), Some("*")));
}

#[test]
fn http_compat_none_producer_is_wildcard() {
    assert!(http_methods_compatible(None, Some("POST")));
}

#[test]
fn http_compat_none_consumer_is_wildcard() {
    assert!(http_methods_compatible(Some("DELETE"), None));
}

#[test]
fn http_compat_both_none_match() {
    assert!(http_methods_compatible(None, None));
}

#[test]
fn http_compat_same_method_match() {
    assert!(http_methods_compatible(Some("GET"), Some("GET")));
    assert!(http_methods_compatible(Some("POST"), Some("POST")));
}

#[test]
fn http_compat_case_insensitive() {
    assert!(http_methods_compatible(Some("get"), Some("GET")));
    assert!(http_methods_compatible(Some("POST"), Some("post")));
}

#[test]
fn http_compat_different_methods_no_match() {
    assert!(!http_methods_compatible(Some("GET"), Some("POST")));
    assert!(!http_methods_compatible(Some("PUT"), Some("PATCH")));
    assert!(!http_methods_compatible(Some("DELETE"), Some("GET")));
}

// ---------------------------------------------------------------------------
// entity_names_match
// ---------------------------------------------------------------------------

#[test]
fn entity_match_exact() {
    assert!(entity_names_match("User", "User"));
}

#[test]
fn entity_match_case_insensitive() {
    assert!(entity_names_match("user", "User"));
    assert!(entity_names_match("USER", "user"));
}

#[test]
fn entity_match_plural_class_to_table() {
    // ORM entity class "User" paired against table key "users".
    assert!(entity_names_match("User", "users"));
}

#[test]
fn entity_match_singular_class_to_plural_table() {
    assert!(entity_names_match("users", "User"));
}

#[test]
fn entity_match_both_plural() {
    assert!(entity_names_match("users", "users"));
}

#[test]
fn entity_match_order_to_orders() {
    assert!(entity_names_match("Order", "orders"));
}

#[test]
fn entity_no_match_different_names() {
    assert!(!entity_names_match("User", "Post"));
    assert!(!entity_names_match("Article", "users"));
}

#[test]
fn entity_no_match_empty() {
    assert!(!entity_names_match("", "User"));
    assert!(!entity_names_match("User", ""));
}

#[test]
fn entity_match_empty_both() {
    // Both empty is technically equal but both would be skipped by the caller.
    assert!(entity_names_match("", ""));
}

// ---------------------------------------------------------------------------
// normalize — template-literal and absolute-URL stripping
// ---------------------------------------------------------------------------

#[test]
fn normalize_strips_template_literal_backticks() {
    assert_eq!(normalize("`/api/users`"), "/api/users");
}

#[test]
fn normalize_strips_scheme_and_host() {
    assert_eq!(normalize("http://localhost:3000/api/users"), "/api/users");
}

#[test]
fn normalize_strips_scheme_host_and_template_backticks() {
    assert_eq!(normalize("`http://localhost:{}/hello`"), "/hello",);
}

#[test]
fn normalize_strips_https_scheme() {
    assert_eq!(
        normalize("https://api.example.com/v1/users/:id"),
        "/v1/users/{}",
    );
}

#[test]
fn normalize_host_only_url_becomes_root() {
    assert_eq!(normalize("http://example.com"), "/");
}

#[test]
fn normalize_strips_leading_template_host_placeholder() {
    // `${baseUrl}/users` → `{}/users` after substitution → `/users` after
    // the host-placeholder strip.
    assert_eq!(normalize("`{}/users`"), "/users");
    assert_eq!(normalize("{}/users"), "/users");
}

#[test]
fn normalize_preserves_path_param_first_segment() {
    // `/{}/posts` — first segment IS a real path parameter (from `:id`
    // captured at the root), NOT a host substitution.  Leading `/` must
    // prevent the host-placeholder strip from firing.
    assert_eq!(normalize("/{}/posts"), "/{}/posts");
}

#[test]
fn normalize_strips_query_string_after_template_replacement() {
    assert_eq!(normalize("`{}/api/font?{}`"), "/api/font",);
}

#[test]
fn normalize_template_with_substitution_in_path() {
    // `${baseUrl}/users/${id}` → `{}/users/{}` → `/users/{}`
    assert_eq!(normalize("`{}/users/{}`"), "/users/{}");
}

// ---------------------------------------------------------------------------
// Goal 50 — cross-language entity-name pairing (strip lang prefix)
// ---------------------------------------------------------------------------

#[test]
fn test_cross_lang_pair_py_rs_user() {
    assert!(entity_names_match("py.User", "rs.User"));
}

#[test]
fn test_cross_lang_pair_cs_java_users() {
    assert!(entity_names_match("cs.Users", "java.users"));
}

#[test]
fn test_cross_lang_pair_php_rb_user() {
    assert!(entity_names_match("php.User", "rb.User"));
}

#[test]
fn test_cross_lang_pair_pluralization_with_prefix() {
    assert!(entity_names_match("kt.Users", "go.User"));
}

#[test]
fn test_cross_lang_wildcard_matches() {
    assert!(entity_names_match("ex.*", "rs.User"));
    assert!(entity_names_match("cs.*", "py.Anything"));
}

#[test]
fn test_cross_lang_no_match_different_entities() {
    assert!(!entity_names_match("py.User", "rs.Post"));
}

#[test]
fn test_cross_lang_pair_case_insensitive() {
    assert!(entity_names_match("scala.users", "ts.Users"));
}

#[test]
fn test_unprefixed_entities_still_pair() {
    assert!(entity_names_match("User", "users"));
    assert!(entity_names_match("User", "USER"));
}
