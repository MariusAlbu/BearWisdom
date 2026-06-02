// Tests for calls.rs — recursive CallArg extraction via real tree-sitter parsing.

use crate::languages::python::extract;
use crate::types::{CallArg, EdgeKind};

/// Parse a Python snippet and return the `call_args` of the first `Calls` ref
/// that carries any arguments.
fn parse_call_args(src: &str) -> Vec<CallArg> {
    let result = extract::extract(src);
    result
        .refs
        .into_iter()
        .find(|r| r.kind == EdgeKind::Calls && !r.call_args.is_empty())
        .map(|r| r.call_args)
        .unwrap_or_default()
}

#[test]
fn call_args_string_literal_preserved() {
    let src = "def caller():\n    fetch('/api/users')\n";
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::StringLit(s) if s == "/api/users")),
        "expected StringLit(\"/api/users\"), got: {args:?}"
    );
}

#[test]
fn call_args_identifier_becomes_ident_variant() {
    let src = "def caller(url):\n    fetch(url)\n";
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Ident(s) if s == "url")),
        "expected Ident(\"url\"), got: {args:?}"
    );
}

#[test]
fn call_args_numeric_literal_preserved() {
    let src = "def caller(cb):\n    schedule(cb, 1000)\n";
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Literal(s) if s == "1000")),
        "expected Literal(\"1000\"), got: {args:?}"
    );
}

#[test]
fn call_args_conditional_expression_produces_ternary_variant() {
    let src = "def caller(a, b, c):\n    f(b if a else c)\n";
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Ternary { .. })),
        "expected Ternary variant for conditional arg, got: {args:?}"
    );
}

#[test]
fn call_args_list_produces_array_literal_variant() {
    let src = "def caller(x, y):\n    f([x, y])\n";
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::ArrayLiteral { .. })),
        "expected ArrayLiteral variant for list arg, got: {args:?}"
    );
}

#[test]
fn call_args_await_produces_await_variant() {
    let src = "async def caller(p):\n    f(await p)\n";
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Await { .. })),
        "expected Await variant for await arg, got: {args:?}"
    );
}

#[test]
fn call_args_list_splat_produces_spread_variant() {
    let src = "def caller(xs):\n    f(*xs)\n";
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Spread { .. })),
        "expected Spread variant for splat arg, got: {args:?}"
    );
}

#[test]
fn call_args_subscript_produces_index_access_variant() {
    let src = "def caller(a, i):\n    f(a[i])\n";
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::IndexAccess { .. })),
        "expected IndexAccess variant for subscript arg, got: {args:?}"
    );
}

#[test]
fn call_args_binary_operator_produces_binary_variant() {
    let src = "def caller(a, b):\n    f(a + b)\n";
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Binary { op, .. } if op == "+")),
        "expected Binary variant with op \"+\" for addition arg, got: {args:?}"
    );
}

#[test]
fn call_args_comparison_operator_produces_binary_variant() {
    let src = "def caller(a, b):\n    f(a == b)\n";
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Binary { op, .. } if op == "==")),
        "expected Binary variant with op \"==\" for comparison arg, got: {args:?}"
    );
}
