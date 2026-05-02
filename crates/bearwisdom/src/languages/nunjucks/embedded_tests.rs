use super::embedded::{_test_strip_pipe_filters, detect_regions};

#[test]
fn expression_becomes_js_region() {
    let src = "<p>{{ currentUser.name }}</p>";
    let regions = detect_regions(src);
    assert!(regions.iter().any(|r| r.text.contains("currentUser.name")));
}

// Regression: pipe-filter syntax must be stripped before the expression is
// embedded as JS, otherwise the JS extractor emits each filter name as a
// Calls ref. Source: jinja-kubespray sweep produced 458 unresolved Calls
// (`indent`, `to_nice_yaml`, `regex_replace`, ...) coming entirely from
// __NjkExpr regions.
#[test]
fn pipe_filter_chain_is_stripped() {
    let src = "{{ thing | list | to_nice_yaml(indent=2) | indent(8) }}";
    let regions = detect_regions(src);
    let js = regions
        .iter()
        .find(|r| r.language_id == "javascript")
        .expect("expected one JS region");
    assert!(js.text.contains("thing"), "leading expression preserved");
    assert!(!js.text.contains("to_nice_yaml"), "filter name dropped");
    assert!(!js.text.contains("indent"), "filter name dropped");
}

#[test]
fn strip_pipe_filters_handles_top_level_pipe() {
    assert_eq!(_test_strip_pipe_filters("a | upper"), "a");
    assert_eq!(_test_strip_pipe_filters("a"), "a");
}

#[test]
fn strip_pipe_filters_preserves_pipe_inside_parens() {
    // The grouping is the expression, not a filter chain.
    assert_eq!(_test_strip_pipe_filters("(a | b)"), "(a | b)");
}

#[test]
fn strip_pipe_filters_preserves_logical_or() {
    assert_eq!(_test_strip_pipe_filters("a || b"), "a || b");
}

#[test]
fn strip_pipe_filters_preserves_pipe_inside_string() {
    assert_eq!(_test_strip_pipe_filters("\"a | b\""), "\"a | b\"");
    assert_eq!(_test_strip_pipe_filters("'a | b'"), "'a | b'");
}

#[test]
fn strip_pipe_filters_clips_at_first_top_level_pipe() {
    let body = "x | regex_replace(pattern, replacement)";
    assert_eq!(_test_strip_pipe_filters(body), "x");
}
