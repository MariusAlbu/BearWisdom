// Tests for calls_args.rs — extract_call_args recursive CallArg variants.

use crate::types::{CallArg, EdgeKind};

/// Parse a Rust source snippet and return the `call_args` of the first
/// `Calls` ref that carries arguments. Mirrors the typescript test harness:
/// the extractor stores `call_args` on the `ExtractedRef` it emits, so we
/// read them back rather than calling the private helper directly.
fn parse_call_args(src: &str) -> Vec<CallArg> {
    let result = super::super::extract::extract(src);
    result
        .refs
        .into_iter()
        .find(|r| r.kind == EdgeKind::Calls && !r.call_args.is_empty())
        .map(|r| r.call_args)
        .unwrap_or_default()
}

fn callback_parameters<'a>(source: &'a str, args: &[CallArg]) -> Vec<Vec<Option<&'a str>>> {
    args.iter()
        .filter_map(|arg| match arg {
            CallArg::LambdaAt { params } => Some(
                params
                    .iter()
                    .map(|span| span.map(|s| &source[s.start as usize..s.end as usize]))
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

#[test]
fn call_args_plain_closure_parameter_uses_its_source_span() {
    let src = r#"
fn caller() { f(|item| item.process()); }
"#;
    assert_eq!(
        callback_parameters(src, &parse_call_args(src)),
        vec![vec![Some("item")]]
    );
}

#[test]
fn call_args_unsupported_closure_patterns_keep_positional_none() {
    let src = r#"
fn caller() { f(|(left, right), mut item, typed: Value| typed.process()); }
"#;
    assert_eq!(
        callback_parameters(src, &parse_call_args(src)),
        vec![vec![None, None, None]]
    );
}

#[test]
fn call_args_array_expression_produces_array_literal_variant() {
    let src = r#"
fn caller(a: i32, b: i32) { f([a, b]); }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::ArrayLiteral { .. })),
        "expected ArrayLiteral variant for array arg, got: {args:?}"
    );
}

#[test]
fn call_args_await_expression_produces_await_variant() {
    let src = r#"
async fn caller(p: Fut) { f(p.await); }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Await { .. })),
        "expected Await variant for await arg, got: {args:?}"
    );
}

#[test]
fn call_args_index_expression_produces_index_access_variant() {
    let src = r#"
fn caller(a: Vec<i32>, i: usize) { f(a[i]); }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::IndexAccess { .. })),
        "expected IndexAccess variant for index arg, got: {args:?}"
    );
}

#[test]
fn call_args_binary_expression_produces_binary_variant() {
    let src = r#"
fn caller(a: i32, b: i32) { f(a + b); }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Binary { op, .. } if op == "+")),
        "expected Binary variant with op \"+\" for addition arg, got: {args:?}"
    );
}

#[test]
fn call_args_simple_string_literal_preserved() {
    let src = r#"
fn caller() { f("/api/users"); }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::StringLit(s) if s == "/api/users")),
        "expected StringLit(\"/api/users\") preserved, got: {args:?}"
    );
}
