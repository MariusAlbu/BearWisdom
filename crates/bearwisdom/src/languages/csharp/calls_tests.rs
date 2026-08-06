// Tests for calls.rs — extract_call_args recursive variant extraction.

use super::extract;
use crate::types::CallArg;

/// Parse a C# snippet and return the `call_args` of the first `Calls` ref that
/// carries any arguments. The args travel via the `ExtractedRef::call_args`
/// the extractor stores, so this exercises the real tree-sitter path rather
/// than calling `extract_arg` against a synthetic node.
fn parse_call_args(source: &str) -> Vec<CallArg> {
    let result = extract::extract(source);
    result
        .refs
        .into_iter()
        .find(|r| r.kind == crate::types::EdgeKind::Calls && !r.call_args.is_empty())
        .map(|r| r.call_args)
        .unwrap_or_default()
}

#[test]
fn call_args_conditional_expression_produces_ternary_variant() {
    let src = r#"
class C { void M(bool cond, int a, int b) { F(cond ? a : b); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Ternary { .. })),
        "expected Ternary variant for conditional arg, got: {args:?}"
    );
}

#[test]
fn call_args_array_creation_produces_array_literal_variant() {
    let src = r#"
class C { void M(int x, int y) { F(new int[] { x, y }); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::ArrayLiteral { elements } if elements.len() == 2)),
        "expected ArrayLiteral with two elements, got: {args:?}"
    );
}

#[test]
fn call_args_implicit_array_creation_produces_array_literal_variant() {
    let src = r#"
class C { void M(int x, int y) { F(new[] { x, y }); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::ArrayLiteral { elements } if elements.len() == 2)),
        "expected ArrayLiteral with two elements, got: {args:?}"
    );
}

#[test]
fn call_args_await_expression_produces_await_variant() {
    let src = r#"
class C { async Task M() { F(await GetValue()); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Await { .. })),
        "expected Await variant for awaited arg, got: {args:?}"
    );
}

#[test]
fn call_args_element_access_produces_index_access_variant() {
    let src = r#"
class C { void M(int[] arr, int i) { F(arr[i]); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(
            a,
            CallArg::IndexAccess { container, index }
                if matches!(**container, CallArg::Ident(ref s) if s == "arr")
                    && matches!(**index, CallArg::Ident(ref s) if s == "i")
        )),
        "expected IndexAccess(arr, i), got: {args:?}"
    );
}

#[test]
fn call_args_binary_expression_produces_binary_variant() {
    let src = r#"
class C { void M(int a, int b) { F(a + b); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Binary { op, .. } if op == "+")),
        "expected Binary variant with `+` operator, got: {args:?}"
    );
}

#[test]
fn call_args_simple_variants_unchanged() {
    // The existing simple-variant behaviour must survive the refactor.
    let src = r#"
class C { void M(string url) { F("/api/users", url, 42, true); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::StringLit(s) if s == "/api/users")),
        "expected StringLit(\"/api/users\"), got: {args:?}"
    );
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Ident(s) if s == "url")),
        "expected Ident(\"url\"), got: {args:?}"
    );
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Literal(s) if s == "42")),
        "expected Literal(\"42\"), got: {args:?}"
    );
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Literal(s) if s == "true")),
        "expected Literal(\"true\"), got: {args:?}"
    );
}

#[test]
fn call_args_lambda_implicit_single_param_captured() {
    // `u => u.Name` — single implicit parameter.
    let src = r#"
class C { void M(System.Collections.Generic.List<int> users) { users.Select(u => u.Name); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params.as_slice() == ["u"])),
        "expected Lambda {{ params: [\"u\"] }}, got: {args:?}"
    );
}

#[test]
fn call_args_lambda_parenthesized_params_captured() {
    // `(x, y) => f(x, y)` — parenthesized parameter list.
    let src = r#"
class C { void M(System.Collections.Generic.List<int> xs) { xs.Select((x, y) => f(x, y)); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params.as_slice() == ["x", "y"])),
        "expected Lambda {{ params: [\"x\", \"y\"] }}, got: {args:?}"
    );
}

#[test]
fn named_argument_unwraps_to_the_value_expression() {
    // A named argument (`columns: table => ...`) wraps the value behind a
    // name-colon node; the capture must skip the name and take the VALUE, or
    // the lambda degrades to Other and its params never seed.
    let src = r#"
class C { void M() { F(name: "Events", columns: table => table); } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::StringLit(s) if s == "Events")),
        "named string arg must capture its value, got: {args:?}"
    );
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params == &["table".to_string()])),
        "named lambda arg must capture its params, got: {args:?}"
    );
}
