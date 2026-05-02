use super::*;

#[test]
fn code_tag_becomes_js_region() {
    let src = "<% const x = getUser(); %>";
    let regions = detect_regions(src);
    assert!(regions.iter().any(|r| r.text.contains("getUser")));
}

#[test]
fn equals_tag_becomes_js_region() {
    let src = "<p><%= userName %></p>";
    let regions = detect_regions(src);
    assert!(regions.iter().any(|r| r.text.contains("userName")));
}

#[test]
fn raw_tag_becomes_js_region() {
    let src = "<%- renderHtml(body) %>";
    let regions = detect_regions(src);
    assert!(regions.iter().any(|r| r.text.contains("renderHtml")));
}

#[test]
fn comment_tag_skipped() {
    let src = "<%# comment content %><p>Hello</p>";
    let regions = detect_regions(src);
    assert!(regions.is_empty());
}

#[test]
fn include_call_replaced_with_null_in_embedded_js() {
    // The host extractor captures include() at the EJS level; the
    // embedded JS should not see `include` as a function call.
    let src = "<%- include('./partials/header') %>";
    let regions = detect_regions(src);
    assert_eq!(regions.len(), 1);
    let body = &regions[0].text;
    assert!(!body.contains("include"), "got body: {body}");
    assert!(body.contains("null"));
}

#[test]
fn strip_include_handles_nested_parens_in_args() {
    let body = "include('./layout', { user: makeUser() }) + 'tail'";
    let stripped = _test_strip_include_calls(body);
    assert_eq!(stripped, "null + 'tail'");
}

#[test]
fn strip_include_preserves_other_calls() {
    let body = "renderHtml(body)";
    let stripped = _test_strip_include_calls(body);
    assert_eq!(stripped, "renderHtml(body)");
}

#[test]
fn strip_include_does_not_match_x_include_prefix() {
    let body = "xinclude('foo')";
    let stripped = _test_strip_include_calls(body);
    assert_eq!(stripped, "xinclude('foo')");
}
