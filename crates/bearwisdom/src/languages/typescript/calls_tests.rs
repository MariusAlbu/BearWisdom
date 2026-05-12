// Tests for calls.rs — extract_call_args and replace_template_substitutions.

use super::calls::replace_template_substitutions;
use crate::types::CallArg;

// ---------------------------------------------------------------------------
// replace_template_substitutions
// ---------------------------------------------------------------------------

#[test]
fn template_no_substitution_passes_through() {
    assert_eq!(replace_template_substitutions("`hello world`"), "`hello world`");
}

#[test]
fn template_single_substitution_replaced() {
    assert_eq!(
        replace_template_substitutions("`/api/${id}`"),
        "`/api/{}`"
    );
}

#[test]
fn template_multiple_substitutions_all_replaced() {
    assert_eq!(
        replace_template_substitutions("`${base}/users/${userId}/profile`"),
        "`{}/users/{}/profile`"
    );
}

#[test]
fn template_nested_braces_inside_substitution() {
    // `${obj.method({ key: val })}` — the `{` inside the expression must not
    // terminate the substitution scan prematurely.
    let input = "`prefix-${obj.method({ key: val })}-suffix`";
    let result = replace_template_substitutions(input);
    assert_eq!(result, "`prefix-{}-suffix`");
}

#[test]
fn template_dollar_without_brace_passes_through() {
    // `$variable` is not a substitution — dollar not followed by `{`.
    let input = "`$PATH:/usr/bin`";
    let result = replace_template_substitutions(input);
    assert_eq!(result, "`$PATH:/usr/bin`");
}

#[test]
fn template_empty_string_returns_empty() {
    assert_eq!(replace_template_substitutions(""), "");
}

// ---------------------------------------------------------------------------
// extract_call_args — via real tree-sitter parsing
// ---------------------------------------------------------------------------

/// Parse a TypeScript expression `fn(<args>)` and return the extracted args.
fn parse_call_args(call_expr_src: &str) -> Vec<CallArg> {
    use crate::languages::typescript::extract;

    let result = extract::extract(call_expr_src, false);
    // We can't call extract_call_args directly without a live Node, so validate
    // via the ExtractedRef call_args that the extractor stores.
    result
        .refs
        .into_iter()
        .find(|r| r.kind == crate::types::EdgeKind::Calls && !r.call_args.is_empty())
        .map(|r| r.call_args)
        .unwrap_or_default()
}

#[test]
fn call_args_string_literal_double_quotes() {
    // Top-level call; extractor should capture the string arg.
    let src = r#"
function caller() { fetch("/api/users"); }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::StringLit(s) if s == "/api/users")),
        "expected StringLit(\"/api/users\"), got: {args:?}"
    );
}

#[test]
fn call_args_string_literal_single_quotes() {
    let src = r#"
function caller() { fetch('/api/posts'); }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::StringLit(s) if s == "/api/posts")),
        "expected StringLit(\"/api/posts\"), got: {args:?}"
    );
}

#[test]
fn call_args_template_literal_with_substitution() {
    let src = r#"
function caller() { fetch(`/api/users/${id}`); }
"#;
    let args = parse_call_args(src);
    // Should be TemplateLit with `{}` placeholder.
    assert!(
        args.iter().any(|a| matches!(a, CallArg::TemplateLit(s) if s.contains("{}"))),
        "expected TemplateLit with placeholder, got: {args:?}"
    );
}

#[test]
fn call_args_identifier_becomes_ident_variant() {
    let src = r#"
function caller(url) { fetch(url); }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Ident(s) if s == "url")),
        "expected Ident(\"url\"), got: {args:?}"
    );
}

#[test]
fn call_args_numeric_literal() {
    let src = r#"
function caller() { setTimeout(cb, 1000); }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Literal(s) if s == "1000")),
        "expected Literal(\"1000\"), got: {args:?}"
    );
}

#[test]
fn call_args_no_args_returns_empty() {
    let src = r#"
function caller() { doThing(); }
"#;
    // The call with no arguments should either have empty call_args or no ref.
    let result = crate::languages::typescript::extract::extract(src, false);
    let ref_with_call = result
        .refs
        .into_iter()
        .find(|r| r.kind == crate::types::EdgeKind::Calls && r.target_name == "doThing");
    if let Some(r) = ref_with_call {
        assert!(r.call_args.is_empty(), "expected empty call_args for no-arg call, got: {:?}", r.call_args);
    }
}
