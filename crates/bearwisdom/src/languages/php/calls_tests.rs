// Tests for calls.rs — extract_call_args recursive CallArg variants.

use crate::languages::php::extract;
use crate::types::CallArg;

/// Parse a PHP source string and return the `call_args` of the first `Calls`
/// ref that carries any (the test snippets each contain one argument-bearing
/// call). PHP requires no declarations, so bare `f($a + $b)` parses cleanly.
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
fn call_args_string_literal_preserved() {
    let src = "<?php fetch('/api/users');";
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::StringLit(s) if s == "/api/users")),
        "expected StringLit(\"/api/users\"), got: {args:?}"
    );
}

#[test]
fn call_args_variable_becomes_ident() {
    let src = "<?php fetch($url);";
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Ident(s) if s == "url")),
        "expected Ident(\"url\"), got: {args:?}"
    );
}

#[test]
fn call_args_conditional_expression_produces_ternary_variant() {
    // `$cond ? $b : $c` — PHP ternary.
    let src = "<?php f($cond ? $b : $c);";
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Ternary { .. })),
        "expected Ternary variant for ternary arg, got: {args:?}"
    );
}

#[test]
fn call_args_short_ternary_produces_ternary_variant() {
    // `$a ?: $b` — short ternary, `body` field absent.
    let src = "<?php f($a ?: $b);";
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Ternary { .. })),
        "expected Ternary variant for short-ternary arg, got: {args:?}"
    );
}

#[test]
fn call_args_array_literal_produces_array_literal_variant() {
    // `[$x, $y]` — array creation as argument.
    let src = "<?php f([$x, $y]);";
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::ArrayLiteral { elements } if elements.len() == 2)),
        "expected ArrayLiteral with 2 elements, got: {args:?}"
    );
}

#[test]
fn call_args_array_function_form_produces_array_literal_variant() {
    // `array($x, $y)` — long array syntax.
    let src = "<?php f(array($x, $y));";
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::ArrayLiteral { .. })),
        "expected ArrayLiteral variant for array() arg, got: {args:?}"
    );
}

#[test]
fn call_args_variadic_unpacking_produces_spread_variant() {
    // `...$xs` — PHP spread / argument unpacking.
    let src = "<?php f(...$xs);";
    let args = parse_call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Spread { .. })),
        "expected Spread variant for variadic-unpacking arg, got: {args:?}"
    );
}

#[test]
fn call_args_subscript_expression_produces_index_access_variant() {
    // `$arr[$i]` — array subscript.
    let src = "<?php f($arr[$i]);";
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::IndexAccess { .. })),
        "expected IndexAccess variant for subscript arg, got: {args:?}"
    );
}

#[test]
fn call_args_binary_expression_produces_binary_variant() {
    // `$a + $b` — arithmetic binary expression.
    let src = "<?php f($a + $b);";
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Binary { op, .. } if op == "+")),
        "expected Binary variant with op \"+\", got: {args:?}"
    );
}

#[test]
fn call_args_string_concat_produces_binary_variant() {
    // `$a . $b` — PHP string concatenation operator.
    let src = "<?php f($a . $b);";
    let args = parse_call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Binary { op, .. } if op == ".")),
        "expected Binary variant with op \".\", got: {args:?}"
    );
}
