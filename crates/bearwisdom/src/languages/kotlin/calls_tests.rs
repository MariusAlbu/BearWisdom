// Tests for calls.rs — extract_call_args recursive variant extraction.

use crate::types::CallArg;

/// Parse a Kotlin snippet and return the args of the first `Calls` ref that
/// carries any captured arguments.
fn parse_call_args(source: &str) -> Vec<CallArg> {
    let result = super::super::extract::extract(source);
    result
        .refs
        .into_iter()
        .find(|r| r.kind == crate::types::EdgeKind::Calls && !r.call_args.is_empty())
        .map(|r| r.call_args)
        .unwrap_or_default()
}

#[test]
fn call_args_string_literal() {
    let src = r#"
fun caller() { fetch("/api/users") }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::StringLit(s) if s == "/api/users")),
        "expected StringLit(\"/api/users\"), got: {args:?}"
    );
}

#[test]
fn call_args_identifier_becomes_ident_variant() {
    let src = r#"
fun caller(url: String) { fetch(url) }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Ident(s) if s == "url")),
        "expected Ident(\"url\"), got: {args:?}"
    );
}

#[test]
fn call_args_if_expression_produces_ternary_variant() {
    // Kotlin's `if` is an expression — the ternary analog.
    let src = r#"
fun caller(c: Boolean, a: Int, b: Int) { f(if (c) a else b) }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Ternary { .. })),
        "expected Ternary variant for if-expression arg, got: {args:?}"
    );
}

#[test]
fn call_args_collection_literal_produces_array_literal_variant() {
    let src = r#"
fun caller(x: Int, y: Int) { f([x, y]) }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::ArrayLiteral { .. })),
        "expected ArrayLiteral variant for collection-literal arg, got: {args:?}"
    );
}

#[test]
fn call_args_spread_expression_produces_spread_variant() {
    let src = r#"
fun caller(xs: IntArray) { f(*xs) }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Spread { .. })),
        "expected Spread variant for spread arg, got: {args:?}"
    );
}

#[test]
fn call_args_index_expression_produces_index_access_variant() {
    let src = r#"
fun caller(a: IntArray, i: Int) { f(a[i]) }
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
fun caller(a: Int, b: Int) { f(a + b) }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Binary { op, .. } if op == "+")),
        "expected Binary variant with op \"+\" for addition arg, got: {args:?}"
    );
}

#[test]
fn call_args_elvis_operator_produces_binary_variant() {
    // Elvis `?:` is modeled as a binary_expression in the grammar.
    let src = r#"
fun caller(a: Int?, b: Int) { f(a ?: b) }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Binary { op, .. } if op == "?:")),
        "expected Binary variant with op \"?:\" for elvis arg, got: {args:?}"
    );
}

#[test]
fn call_args_trailing_lambda_named_param_captured() {
    // `list.map { x -> x.foo }` — the trailing lambda is an `annotated_lambda`
    // sibling of the call_expression, not a value_argument.
    let src = r#"
fun caller(list: List<Int>) { list.map { x -> x.foo } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params.as_slice() == ["x"])),
        "expected Lambda {{ params: [\"x\"] }}, got: {args:?}"
    );
}

#[test]
fn call_args_trailing_lambda_implicit_it_synthesized() {
    // `list.map { it.foo }` — no `lambda_parameters`; the implicit single
    // parameter `it` is synthesized so the seed key exists.
    let src = r#"
fun caller(list: List<Int>) { list.map { it.foo } }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params.as_slice() == ["it"])),
        "expected Lambda {{ params: [\"it\"] }}, got: {args:?}"
    );
}

#[test]
fn call_args_parenthesized_lambda_param_captured() {
    // `list.map({ y -> y.foo })` — the lambda is a value_argument here.
    let src = r#"
fun caller(list: List<Int>) { list.map({ y -> y.foo }) }
"#;
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params.as_slice() == ["y"])),
        "expected Lambda {{ params: [\"y\"] }}, got: {args:?}"
    );
}
