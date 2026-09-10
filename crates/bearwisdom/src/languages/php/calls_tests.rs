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

fn calls(source: &str) -> Vec<crate::types::ExtractedRef> {
    refs(source)
        .into_iter()
        .filter(|reference| reference.kind == crate::types::EdgeKind::Calls)
        .collect()
}

fn refs(source: &str) -> Vec<crate::types::ExtractedRef> {
    extract::extract(source).refs
}

fn chain_root(reference: &crate::types::ExtractedRef) -> Option<&str> {
    reference
        .chain
        .as_ref()
        .and_then(|chain| chain.segments.first())
        .map(|segment| segment.name.as_str())
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

#[test]
fn arrow_function_callback_parameters_keep_exact_variable_spans() {
    let src = "<?php visit(fn (Item $item, ?Other $other) => $item->touch());";

    assert_eq!(
        callback_parameters(src, &parse_call_args(src)),
        vec![vec![Some("$item"), Some("$other")]]
    );
}

#[test]
fn anonymous_function_callback_parameters_keep_exact_variable_spans() {
    let src = "<?php visit(function (Item $item, ...$rest) { $item->touch(); });";

    assert_eq!(
        callback_parameters(src, &parse_call_args(src)),
        vec![vec![Some("$item"), None]]
    );
}

#[test]
fn callback_parameter_holes_preserve_later_php_parameter_positions() {
    // Property-promotion parameters are a distinct formal-parameter node
    // shape. They are unsupported in callback position, so `$item` must stay
    // in position two instead of shifting into position one.
    let src = "<?php visit(fn (public Item $ignored, $item) => $item->touch());";

    assert_eq!(
        callback_parameters(src, &parse_call_args(src)),
        vec![vec![None, Some("$item")]]
    );
}

#[test]
fn callback_expression_and_block_bodies_emit_member_calls_once() {
    for (source, callback) in [
        (
            "<?php $xs->map(fn (Item $item) => $item->touch());",
            vec![Some("$item")],
        ),
        (
            "<?php $xs->map(function (Item $item) { $item->touch(); });",
            vec![Some("$item")],
        ),
    ] {
        let refs = calls(source);
        let maps: Vec<_> = refs
            .iter()
            .filter(|reference| reference.target_name == "map")
            .collect();
        assert_eq!(
            maps.len(),
            1,
            "outer map must be emitted exactly once: {refs:?}"
        );
        assert_eq!(chain_root(maps[0]), Some("xs"));
        assert_eq!(
            callback_parameters(source, &maps[0].call_args),
            vec![callback],
            "the outer call must retain its LambdaAt argument"
        );

        let touches: Vec<_> = refs
            .iter()
            .filter(|reference| reference.target_name == "touch")
            .collect();
        assert_eq!(
            touches.len(),
            1,
            "the callback body member call must be emitted exactly once: {refs:?}"
        );
        assert_eq!(chain_root(touches[0]), Some("item"));
    }
}

#[test]
fn nested_arrow_callbacks_emit_inner_call_and_body_member_once() {
    let source =
        "<?php $xs->map(fn (Item $item) => $ys->map(fn (Other $other) => $other->inner()));";
    let refs = calls(source);
    let maps: Vec<_> = refs
        .iter()
        .filter(|reference| reference.target_name == "map")
        .collect();
    assert_eq!(
        maps.len(),
        2,
        "both map calls must be emitted once: {refs:?}"
    );

    let outer = maps
        .iter()
        .find(|reference| chain_root(reference) == Some("xs"))
        .expect("outer xs.map call");
    let inner = maps
        .iter()
        .find(|reference| chain_root(reference) == Some("ys"))
        .expect("inner ys.map call");
    assert_eq!(
        callback_parameters(source, &outer.call_args),
        vec![vec![Some("$item")]]
    );
    assert_eq!(
        callback_parameters(source, &inner.call_args),
        vec![vec![Some("$other")]]
    );

    let inner_refs: Vec<_> = refs
        .iter()
        .filter(|reference| reference.target_name == "inner")
        .collect();
    assert_eq!(
        inner_refs.len(),
        1,
        "nested callback body member call must be emitted exactly once: {refs:?}"
    );
    assert_eq!(chain_root(inner_refs[0]), Some("other"));
}

#[test]
fn arrow_callback_body_emits_direct_instantiation_once() {
    let source = "<?php $xs->map(fn (Item $item) => new Foo());";
    let refs = refs(source);

    let maps: Vec<_> = refs
        .iter()
        .filter(|reference| {
            reference.kind == crate::types::EdgeKind::Calls && reference.target_name == "map"
        })
        .collect();
    assert_eq!(maps.len(), 1, "outer map must be emitted once: {refs:?}");
    assert_eq!(
        callback_parameters(source, &maps[0].call_args),
        vec![vec![Some("$item")]]
    );

    let creations: Vec<_> = refs
        .iter()
        .filter(|reference| {
            reference.kind == crate::types::EdgeKind::Instantiates && reference.target_name == "Foo"
        })
        .collect();
    assert_eq!(
        creations.len(),
        1,
        "direct arrow callback instantiation must be emitted once: {refs:?}"
    );
}

#[test]
fn anonymous_callback_body_emits_include_require_once() {
    let source = "<?php $xs->map(function () { require 'config.php'; include_once 'support/helpers.php'; });";
    let refs = refs(source);

    let imports: Vec<_> = refs
        .iter()
        .filter(|reference| reference.kind == crate::types::EdgeKind::Imports)
        .map(|reference| (reference.target_name.as_str(), reference.module.as_deref()))
        .collect();
    assert_eq!(
        imports,
        vec![("config", None), ("helpers", Some("support"))]
    );
}

#[test]
fn nested_callback_body_emits_instantiation_and_import_once() {
    let source = "<?php $xs->map(fn (Item $item) => $ys->map(function () { require 'nested.php'; new Foo(); }));";
    let refs = refs(source);

    let maps: Vec<_> = refs
        .iter()
        .filter(|reference| {
            reference.kind == crate::types::EdgeKind::Calls && reference.target_name == "map"
        })
        .collect();
    assert_eq!(maps.len(), 2, "nested maps must be emitted once: {refs:?}");

    let creations = refs
        .iter()
        .filter(|reference| {
            reference.kind == crate::types::EdgeKind::Instantiates && reference.target_name == "Foo"
        })
        .count();
    assert_eq!(
        creations, 1,
        "nested creation must be emitted once: {refs:?}"
    );

    let imports: Vec<_> = refs
        .iter()
        .filter(|reference| reference.kind == crate::types::EdgeKind::Imports)
        .map(|reference| reference.target_name.as_str())
        .collect();
    assert_eq!(imports, vec!["nested"]);
}
