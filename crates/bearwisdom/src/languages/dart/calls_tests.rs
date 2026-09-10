// Tests for calls.rs — extract_dart_call_args recursive CallArg construction.

use super::extract_dart_call_args;
use crate::types::{CallArg, EdgeKind};
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

fn extracted_calls(src: &str) -> Vec<crate::types::ExtractedRef> {
    super::super::extract::extract(src)
        .refs
        .into_iter()
        .filter(|reference| reference.kind == EdgeKind::Calls)
        .collect()
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
fn call_args_function_expression_params_use_exact_declaration_spans() {
    let src = "void caller(xs) { xs.map((u) => u.name); }";
    assert_eq!(
        callback_parameters(src, &call_args(src)),
        vec![vec![Some("u")]]
    );
}

#[test]
fn call_args_function_expression_multi_param_spans_preserve_order() {
    let src = "void caller(xs) { xs.fold((a, b) => f(a, b)); }";
    assert_eq!(
        callback_parameters(src, &call_args(src)),
        vec![vec![Some("a"), Some("b")]]
    );
}

#[test]
fn call_args_function_expression_wildcard_keeps_a_positional_hole() {
    let src = "void caller(xs) { xs.map((_) => 0); }";
    assert_eq!(callback_parameters(src, &call_args(src)), vec![vec![None]]);
}

#[test]
fn postfix_member_call_keeps_receiver_chain_and_source_anchor() {
    let src = "void caller(Item item) { item.touch(); }";
    let calls = extracted_calls(src);
    let touches: Vec<_> = calls
        .iter()
        .filter(|reference| reference.target_name == "touch")
        .collect();

    assert_eq!(touches.len(), 1, "expected one item.touch call: {calls:?}");
    let touch = touches[0];
    assert_eq!(
        touch.chain.as_ref().map(|chain| chain
            .segments
            .iter()
            .map(|segment| segment.name.as_str())
            .collect::<Vec<_>>()),
        Some(vec!["item", "touch"]),
        "member call must retain its receiver root: {touch:?}"
    );
    let chain = touch.chain.as_ref().expect("member call chain");
    let receiver_offset = src.find("item.touch").expect("receiver call source");
    assert_eq!(chain.segments[0].byte_offset as usize, receiver_offset);
    assert_eq!(
        chain.segments[1].byte_offset as usize,
        receiver_offset + "item.".len()
    );
    assert!(chain.segments[1].is_call, "terminal selector is invoked");
    assert!(chain.segments[1].call_args.is_empty());
    assert_eq!(
        touch.byte_offset as usize, receiver_offset,
        "callback lexical capture must be anchored at the receiver call"
    );
}

#[test]
fn postfix_member_call_retains_callback_argument() {
    let src = "void caller(List<Item> items) { items.map((item) => item.touch()); }";
    let calls = extracted_calls(src);
    let map = calls
        .iter()
        .find(|reference| reference.target_name == "map")
        .expect("map call");
    let chain = map.chain.as_ref().expect("map receiver chain");
    let receiver_offset = src.find("items.map").expect("map receiver source");
    assert_eq!(chain.segments[0].byte_offset as usize, receiver_offset);
    assert_eq!(
        chain.segments[1].byte_offset as usize,
        receiver_offset + "items.".len()
    );
    assert!(chain.segments[1].is_call, "map selector is invoked");
    assert_eq!(
        callback_parameters(src, &map.call_args),
        vec![vec![Some("item")]]
    );
    assert_eq!(
        callback_parameters(src, &chain.segments[1].call_args),
        vec![vec![Some("item")]],
        "the invoked chain segment owns its callback arguments"
    );
}

#[test]
fn conditional_member_call_marks_the_terminal_optional() {
    let src = "void caller(item) { item?.touch(); }";
    let calls = extracted_calls(src);
    let touch = calls
        .iter()
        .find(|reference| reference.target_name == "touch")
        .expect("conditional touch call");
    let chain = touch.chain.as_ref().expect("conditional member chain");
    assert!(
        chain
            .segments
            .last()
            .is_some_and(|segment| segment.optional_chaining),
        "conditional selector must not become an unconditional member walk: {chain:?}"
    );
}

#[test]
fn indexed_receiver_member_call_declines_an_incomplete_chain() {
    let src = "void caller(List<Item> items) { items[0].touch(); }";
    let calls = extracted_calls(src);
    let touch = calls
        .iter()
        .find(|reference| reference.target_name == "touch")
        .expect("indexed touch call");
    assert!(
        touch.chain.is_none(),
        "an indexed receiver must not be shortened to items.touch: {touch:?}"
    );
}

#[test]
fn expression_bodied_nested_callback_emits_its_member_call_once() {
    let src = "void f(List<A> xs, List<B> ys) { xs.map((A x) { ys.map((B x) => x.inner()); x.outer(); }); }";
    let calls = extracted_calls(src);

    for (target, expected_segments) in
        [("inner", vec!["x", "inner"]), ("outer", vec!["x", "outer"])]
    {
        let matches: Vec<_> = calls
            .iter()
            .filter(|reference| reference.target_name == target)
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "expected exactly one x.{target} call: {calls:?}"
        );
        assert_eq!(
            matches[0].chain.as_ref().map(|chain| chain
                .segments
                .iter()
                .map(|segment| segment.name.as_str())
                .collect::<Vec<_>>()),
            Some(expected_segments),
            "expected receiver chain for x.{target}: {:?}",
            matches[0]
        );
    }
}
