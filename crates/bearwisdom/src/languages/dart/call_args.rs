// =============================================================================
// dart/call_args.rs — call-site argument evidence for Dart
//
// Turns the argument list of a call into `CallArg` values: literals, locals,
// lambdas with their parameter spans, index accesses and binary expressions,
// bounded in nesting depth.
// =============================================================================

use super::helpers::node_text;
use crate::types::{CallArg, SourceSpan};
use tree_sitter::Node;

/// Maximum nesting depth for recursive `CallArg` construction. Arguments
/// deeper than this collapse to `CallArg::Other` rather than recursing further.
const MAX_ARG_DEPTH: u32 = 8;

pub(super) fn extract_dart_call_args(call_node: &Node, src: &str) -> Vec<CallArg> {
    let mut args_node: Option<Node> = None;
    let mut cursor = call_node.walk();
    for c in call_node.children(&mut cursor) {
        if matches!(c.kind(), "arguments" | "argument_part" | "argument_list") {
            args_node = Some(c);
            break;
        }
        // Selector wrapper: postfix_expression → selector → argument_part.
        if c.kind() == "selector" {
            let mut sc = c.walk();
            for s in c.children(&mut sc) {
                if s.kind() == "argument_part" {
                    let mut ac = s.walk();
                    for a in s.children(&mut ac) {
                        if a.kind() == "arguments" {
                            args_node = Some(a);
                        }
                    }
                }
            }
        }
    }
    let Some(args) = args_node else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut ac = args.walk();
    for child in args.named_children(&mut ac) {
        out.push(extract_arg(&child, src, 0));
    }
    out
}

/// Convert a single Dart argument expression node to a `CallArg`, recursing
/// for composite expression kinds up to `MAX_ARG_DEPTH`.
///
/// Dart wraps each positional/named argument in an `argument` (and named ones
/// add a `named_argument`) node; both are pass-through to the inner
/// expression. Binary operators carry no field names, so operands are read as
/// the first/last named children and the operator text is the source slice
/// between them. Subscript access (`a[i]`) has no dedicated node — it surfaces
/// as a postfix `primary` + `selector(unconditional_assignable_selector)`
/// sibling pair inside the wrapper, handled by `index_access_from_postfix`.
fn extract_arg(node: &Node, src: &str, depth: u32) -> CallArg {
    if depth >= MAX_ARG_DEPTH {
        return CallArg::Other;
    }
    match node.kind() {
        "string_literal" | "adjacent_string_literals" => {
            let raw = node_text(*node, src);
            CallArg::StringLit(raw.trim_matches('\'').trim_matches('"').to_string())
        }
        "identifier" => CallArg::Ident(node_text(*node, src)),
        "decimal_integer_literal" | "double_literal" | "integer_literal" => {
            CallArg::Literal(node_text(*node, src))
        }
        "boolean_literal" | "null_literal" => CallArg::Literal(node.kind().to_string()),

        // `argument` / `named_argument` wrap the real expression; `named_argument`
        // also holds a leading `label`. Both are transparent — but a wrapper
        // whose children are `primary` + index `selector` is a subscript
        // (`a[i]`), which has no node of its own.
        "argument" | "named_argument" => {
            if let Some(idx) = index_access_from_postfix(node, src, depth) {
                return idx;
            }
            let inner = first_value_child(node);
            inner
                .map(|n| extract_arg(&n, src, depth))
                .unwrap_or(CallArg::Other)
        }

        // `cond ? then : else` — the condition is the unnamed leading child;
        // only the two value branches are preserved.
        "conditional_expression" => {
            let then_branch = node
                .child_by_field_name("consequence")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let else_branch = node
                .child_by_field_name("alternative")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Ternary {
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            }
        }

        // `[a, b, ...c]` — recurse on each `_element` child. Spread elements
        // become `CallArg::Spread` children; `type_arguments` are skipped.
        "list_literal" => {
            let mut cursor = node.walk();
            let elements = node
                .named_children(&mut cursor)
                .filter(|c| c.kind() != "type_arguments")
                .map(|c| extract_arg(&c, src, depth + 1))
                .collect();
            CallArg::ArrayLiteral { elements }
        }

        // `await x`, `-x`, `!x` — Dart nests the operand (and an `await` arg's
        // `await_expression`) under a `unary_expression`. The value's type is
        // the operand's, so descend to the last named child.
        "unary_expression" => node
            .named_child(node.named_child_count().saturating_sub(1))
            .map(|n| extract_arg(&n, src, depth + 1))
            .unwrap_or(CallArg::Other),

        // `await expr` — the awaited expression is the sole named child.
        "await_expression" => {
            let inner = node
                .named_child(0)
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Await {
                expr: Box::new(inner),
            }
        }

        // `...expr` / `...?expr` — the operand is the `value` field.
        "spread_element" => {
            let inner = node
                .child_by_field_name("value")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Spread {
                expr: Box::new(inner),
            }
        }

        // Binary expressions carry no operand fields — the operator is an
        // anonymous token between the first and last named operands.
        "additive_expression"
        | "multiplicative_expression"
        | "relational_expression"
        | "equality_expression"
        | "logical_and_expression"
        | "logical_or_expression"
        | "if_null_expression"
        | "bitwise_or_expression"
        | "bitwise_and_expression"
        | "bitwise_xor_expression"
        | "shift_expression" => binary_from_node(node, src, depth),

        // `(u) => u.name`, `(a, b) => f(a, b)` — capture the closure's own
        // positional parameter declaration spans so the chain walker can type
        // them from the higher-order method's callback-parameter signature.
        "function_expression" => CallArg::LambdaAt {
            params: dart_lambda_param_spans(node, src),
        },

        _ => CallArg::Other,
    }
}

/// Collect the positional parameter declaration spans of a Dart
/// `function_expression` argument. The `parameters` field is a
/// `formal_parameter_list` of `formal_parameter` nodes wrapping an
/// `identifier`. A parameter without a plain identifier yields a `None` slot so
/// positions stay aligned with the callback signature.
fn dart_lambda_param_spans(node: &Node, src: &str) -> Vec<Option<SourceSpan>> {
    let Some(params) = node.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut cursor = params.walk();
    params
        .named_children(&mut cursor)
        .filter(|p| p.kind() == "formal_parameter")
        .map(|p| dart_plain_parameter_span(&p, src))
        .collect()
}

/// The declaration span of a plain identifier `formal_parameter`. A wildcard
/// (`_`) has no durable local binding, so it remains a positional hole instead
/// of borrowing a source token as identity evidence.
fn dart_plain_parameter_span(node: &Node, src: &str) -> Option<SourceSpan> {
    let mut i = 0;
    while let Some(child) = node.named_child(i) {
        if child.kind() == "identifier" {
            return (node_text(child, src) != "_").then(|| SourceSpan {
                start: child.start_byte() as u32,
                end: child.end_byte() as u32,
            });
        }
        i += 1;
    }
    None
}

/// Detect a subscript argument (`a[i]`). Dart has no `index_expression` node:
/// `a[i]` parses as a postfix `primary` followed by a `selector` holding an
/// `unconditional_assignable_selector` of the form `[ expr ]`. When `node`'s
/// named children are exactly `[container, selector([index])]`, build an
/// `IndexAccess`; otherwise return `None` so the caller falls through.
fn index_access_from_postfix(node: &Node, src: &str, depth: u32) -> Option<CallArg> {
    let mut cursor = node.walk();
    let children: Vec<Node> = node
        .named_children(&mut cursor)
        .filter(|c| c.kind() != "label")
        .collect();
    if children.len() != 2 {
        return None;
    }
    let container = children[0];
    let selector = children[1];
    if selector.kind() != "selector" {
        return None;
    }
    let inner = selector.named_child(0)?;
    if inner.kind() != "unconditional_assignable_selector" {
        return None;
    }
    // The `[ expr ]` form starts with `[`; the `.member` form starts with `.`.
    if !node_text(inner, src).trim_start().starts_with('[') {
        return None;
    }
    let index = inner.named_child(0)?;
    Some(CallArg::IndexAccess {
        container: Box::new(extract_arg(&container, src, depth + 1)),
        index: Box::new(extract_arg(&index, src, depth + 1)),
    })
}

/// Build a `CallArg::Binary` from a binary-operator node. The operator text is
/// the source slice between the first and last named operands.
fn binary_from_node(node: &Node, src: &str, depth: u32) -> CallArg {
    let mut cursor = node.walk();
    let operands: Vec<Node> = node.named_children(&mut cursor).collect();
    if operands.len() < 2 {
        return CallArg::Other;
    }
    let left = operands[0];
    let right = operands[operands.len() - 1];
    let op = src
        .get(left.end_byte()..right.start_byte())
        .unwrap_or("")
        .trim()
        .to_string();
    CallArg::Binary {
        op,
        left: Box::new(extract_arg(&left, src, depth + 1)),
        right: Box::new(extract_arg(&right, src, depth + 1)),
    }
}

/// Return the first named child of a wrapper that carries the argument value,
/// skipping a leading `label` on a `named_argument`.
fn first_value_child<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut i = 0;
    while let Some(child) = node.named_child(i) {
        if child.kind() != "label" {
            return Some(child);
        }
        i += 1;
    }
    None
}
