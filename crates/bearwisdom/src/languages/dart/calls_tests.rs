// Tests for calls.rs — extract_dart_call_args recursive CallArg construction.

use super::extract_dart_call_args;
use crate::types::CallArg;
use tree_sitter::{Node, Parser, Tree};

fn parse(src: &str) -> Tree {
    let language: tree_sitter::Language = tree_sitter_dart::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&language).expect("load Dart grammar");
    parser.parse(src, None).expect("parse Dart")
}

/// Depth-first search for the first node of `kind`.
fn find<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find(child, kind) {
            return Some(found);
        }
    }
    None
}

/// Parse a Dart snippet, locate the first `argument_part` (the `(args)` of a
/// call), and run the call-arg extractor against it.
fn call_args(src: &str) -> Vec<CallArg> {
    let tree = parse(src);
    let arg_part = find(tree.root_node(), "argument_part").expect("argument_part node in snippet");
    extract_dart_call_args(&arg_part, src)
}

#[test]
fn call_args_string_literal() {
    let src = "void caller() { fetch('/api/users'); }";
    let args = call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::StringLit(s) if s == "/api/users")),
        "expected StringLit, got: {args:?}"
    );
}

#[test]
fn call_args_identifier() {
    let src = "void caller(x) { f(x); }";
    let args = call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Ident(s) if s == "x")),
        "expected Ident, got: {args:?}"
    );
}

#[test]
fn call_args_conditional_produces_ternary_variant() {
    let src = "void caller(a, b, c) { f(a ? b : c); }";
    let args = call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Ternary { .. })),
        "expected Ternary variant for conditional arg, got: {args:?}"
    );
}

#[test]
fn call_args_list_literal_produces_array_literal_variant() {
    let src = "void caller(x, y) { f([x, y]); }";
    let args = call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::ArrayLiteral { .. })),
        "expected ArrayLiteral variant for list arg, got: {args:?}"
    );
}

#[test]
fn call_args_await_expression_produces_await_variant() {
    let src = "Future<void> caller(p) async { f(await p); }";
    let args = call_args(src);
    assert!(
        args.iter().any(|a| matches!(a, CallArg::Await { .. })),
        "expected Await variant for await arg, got: {args:?}"
    );
}

#[test]
fn call_args_spread_element_produces_spread_variant() {
    let src = "void caller(xs) { f([...xs]); }";
    let args = call_args(src);
    // The spread is an element of the list argument; recurse into it.
    let spread_seen = args.iter().any(|a| match a {
        CallArg::Spread { .. } => true,
        CallArg::ArrayLiteral { elements } => {
            elements.iter().any(|e| matches!(e, CallArg::Spread { .. }))
        }
        _ => false,
    });
    assert!(
        spread_seen,
        "expected Spread variant for spread element, got: {args:?}"
    );
}

#[test]
fn call_args_subscript_produces_index_access_variant() {
    let src = "void caller(a, i) { f(a[i]); }";
    let args = call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::IndexAccess { .. })),
        "expected IndexAccess variant for subscript arg, got: {args:?}"
    );
}

#[test]
fn call_args_binary_expression_produces_binary_variant() {
    let src = "void caller(a, b) { f(a + b); }";
    let args = call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Binary { op, .. } if op == "+")),
        "expected Binary variant with op \"+\", got: {args:?}"
    );
}

#[test]
fn call_args_function_expression_single_param_captured() {
    let src = "void caller(xs) { xs.map((u) => u.name); }";
    let args = call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params.as_slice() == ["u"])),
        "expected Lambda {{ params: [\"u\"] }}, got: {args:?}"
    );
}

#[test]
fn call_args_function_expression_multi_param_captured() {
    let src = "void caller(xs) { xs.fold((a, b) => f(a, b)); }";
    let args = call_args(src);
    assert!(
        args.iter()
            .any(|a| matches!(a, CallArg::Lambda { params } if params.as_slice() == ["a", "b"])),
        "expected Lambda {{ params: [\"a\", \"b\"] }}, got: {args:?}"
    );
}
