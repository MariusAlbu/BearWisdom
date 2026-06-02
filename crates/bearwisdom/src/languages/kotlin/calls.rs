// =============================================================================
// kotlin/calls.rs  —  Call extraction and member chain builder for Kotlin
// =============================================================================

use super::decorators::extract_when_patterns;
use super::helpers::{call_target_name, node_text};
use crate::types::{CallArg, ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

#[cfg(test)]
#[path = "calls_tests.rs"]
mod tests;

/// Maximum nesting depth for recursive `CallArg` construction. Arguments
/// deeper than this collapse to `CallArg::Other` rather than recursing further.
const MAX_ARG_DEPTH: u32 = 32;

/// Extract positional args from a Kotlin `call_expression` / navigation call.
/// Captures string-literal, identifier, integer/float, boolean literals plus
/// the recursive expression shapes (`if`-as-ternary, collection literal, spread,
/// index access, binary). Tree-sitter-kotlin-ng wraps args in `value_arguments`
/// with nested `value_argument` children that contain the expression.
pub(super) fn extract_call_args(call_node: &Node, src: &[u8]) -> Vec<CallArg> {
    let mut args_node: Option<Node> = None;
    let mut cursor = call_node.walk();
    for c in call_node.children(&mut cursor) {
        if c.kind() == "value_arguments" {
            args_node = Some(c);
            break;
        }
    }
    let Some(args) = args_node else { return Vec::new() };
    let mut out = Vec::new();
    let mut ac = args.walk();
    for child in args.named_children(&mut ac) {
        // A `value_argument` is `[name =] [*] expression`: the optional name
        // label and the optional spread `*` are absorbed at this level (a bare
        // `*` token, not a `spread_expression` node), so detect the spread here.
        let (value_node, is_spread) = if child.kind() == "value_argument" {
            let spread = (0..child.child_count())
                .any(|i| child.child(i).map(|c| c.kind() == "*").unwrap_or(false));
            let mut found: Option<Node> = None;
            let mut vc = child.walk();
            for v in child.named_children(&mut vc) {
                if v.kind() != "simple_identifier" || found.is_some() {
                    found = Some(v);
                    break;
                }
                found = Some(v);
            }
            match found {
                Some(n) => (n, spread),
                None => continue,
            }
        } else {
            (child, false)
        };
        let arg = extract_arg(&value_node, src, 0);
        out.push(if is_spread {
            CallArg::Spread { expr: Box::new(arg) }
        } else {
            arg
        });
    }
    out
}

/// Convert a single Kotlin expression node to a `CallArg`, recursing for
/// composite expression kinds up to `MAX_ARG_DEPTH`.
fn extract_arg(node: &Node, src: &[u8], depth: u32) -> CallArg {
    if depth >= MAX_ARG_DEPTH {
        return CallArg::Other;
    }
    match node.kind() {
        "string_literal" | "line_string_literal" | "multi_line_string_literal" => {
            let raw = node_text(*node, src);
            CallArg::StringLit(strip_kt_string(&raw))
        }
        "simple_identifier" | "identifier" => CallArg::Ident(node_text(*node, src)),
        "integer_literal" | "long_literal" | "real_literal" | "hex_literal"
        | "bin_literal" | "unsigned_literal" => CallArg::Literal(node_text(*node, src)),
        "boolean_literal" | "null_literal" => CallArg::Literal(node.kind().to_string()),

        // `if (cond) a else b` — Kotlin's `if` is an expression and the direct
        // analog of a ternary: the condition does not affect the value type, so
        // only the two value branches are preserved. The `condition` field is
        // discarded; the remaining named children are the then/else branches in
        // source order.
        "if_expression" => {
            let cond = node.child_by_field_name("condition");
            let mut branches: Vec<Node> = Vec::new();
            let mut bc = node.walk();
            for ch in node.named_children(&mut bc) {
                if Some(ch) == cond {
                    continue;
                }
                branches.push(ch);
            }
            let then_branch = branches
                .first()
                .map(|n| extract_arg(n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let else_branch = branches
                .get(1)
                .map(|n| extract_arg(n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Ternary {
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            }
        }

        // `[a, b, c]` — collection literal. Recurse on each element expression.
        "collection_literal" => {
            let mut cursor = node.walk();
            let elements = node
                .named_children(&mut cursor)
                .map(|child| extract_arg(&child, src, depth + 1))
                .collect();
            CallArg::ArrayLiteral { elements }
        }

        // `*xs` — spread operand. The sole named child is the spread expression.
        "spread_expression" => {
            let inner = node
                .named_child(0)
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Spread { expr: Box::new(inner) }
        }

        // `container[index]` — first named child is the container, the second
        // is the (first) index expression. Multi-index `a[i, j]` keeps only the
        // first index, which is all the `IndexAccess` variant carries.
        "index_expression" => {
            let container = node
                .named_child(0)
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let index = node
                .named_child(1)
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::IndexAccess {
                container: Box::new(container),
                index: Box::new(index),
            }
        }

        // `left op right` — capture operator text and recurse on operands. The
        // Elvis operator (`a ?: b`) lands here too, as `binary_expression` with
        // operator `?:`.
        "binary_expression" => {
            let op = node
                .child_by_field_name("operator")
                .map(|n| node_text(n, src))
                .unwrap_or_default();
            let left = node
                .child_by_field_name("left")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let right = node
                .child_by_field_name("right")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            }
        }

        _ => CallArg::Other,
    }
}

fn strip_kt_string(raw: &str) -> String {
    let s = raw.trim();
    let s = s.trim_start_matches("\"\"\"").trim_end_matches("\"\"\"");
    s.trim_start_matches('"').trim_end_matches('"').to_string()
}

pub(super) fn extract_calls_from_body(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call_expression" => {
                if let Some(callee) = child.named_child(0) {
                    let chain = build_chain(&callee, src);
                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| call_target_name(&callee, src));
                    let call_args = extract_call_args(&child, src);
                    crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &callee, refs);
                    if !target_name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: callee.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain,
                            byte_offset: callee.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args,
                        });
                    }
                }
                // Recurse only into argument nodes — NOT into the callee itself.
                // The chain builder already captures the full callee chain, and
                // recursing into the callee's navigation_expression would re-emit
                // every intermediate chain segment as a Calls ref.
                extract_call_arguments(&child, src, source_symbol_index, refs);
            }

            // `obj.method` or `obj.method()` — emit Calls for the navigation
            // expression when it is not itself wrapped in a call_expression.
            // `call_expression` already handles the `call_expression(navigation_expression)`
            // pattern; this arm catches standalone navigation access.
            //
            // Do NOT recurse into the navigation expression itself — `build_chain`
            // already traverses the full receiver chain, and recursion would
            // re-emit every nested navigation_expression's last segment as a Calls
            // ref (e.g. `com.foo.bar.X.method` would emit `foo`, `bar`, `X`,
            // `method` as separate Calls refs instead of just `method`).
            "navigation_expression" => {
                let chain = build_chain(&child, src);
                let call_args = extract_call_args(&child, src);
                if let Some(ref c) = chain {
                    if let Some(seg) = c.segments.last() {
                        let target = seg.name.clone();
                        if !target.is_empty() {
                            crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &child, refs);
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: target,
                                kind: EdgeKind::Calls,
                                line: child.start_position().row as u32,
                                col: 0,
                                module: None,
                                chain,
                                byte_offset: child.start_byte() as u32,
                                namespace_segments: Vec::new(),
                                call_args,
                            });
                        }
                    }
                }
                // Only recurse into argument-carrying children (value_arguments,
                // lambda_literal), not into the chain itself.
                extract_nav_arguments(&child, src, source_symbol_index, refs);
            }

            // Extract TypeRef edges from `is` checks inside when expressions.
            "when_expression" => {
                extract_when_patterns(&child, src, source_symbol_index, refs);
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `x as Type` — emit TypeRef for the cast type (field: right).
            "as_expression" => {
                if let Some(type_node) = child.child_by_field_name("right") {
                    extract_type_ref_from_type_node(&type_node, src, source_symbol_index, refs);
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `x is Type` — emit TypeRef for the checked type (field: right).
            "is_expression" => {
                if let Some(type_node) = child.child_by_field_name("right") {
                    extract_type_ref_from_type_node(&type_node, src, source_symbol_index, refs);
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // Type references in variable declarations, return types, local vars.
            "user_type" | "nullable_type" => {
                extract_type_ref_from_type_node(&child, src, source_symbol_index, refs);
                // user_type can contain type_arguments — recurse so generic args
                // (e.g. `List<MyClass>`) also emit TypeRef edges.
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // Generic type arguments: `List<String>`, `Map<K, V>`, etc.
            // Extract TypeRefs from inside the angle brackets.
            "type_arguments" => {
                let mut tc = child.walk();
                for arg in child.children(&mut tc) {
                    match arg.kind() {
                        "type" | "user_type" | "nullable_type" | "function_type"
                        | "non_nullable_type" | "parenthesized_type" => {
                            extract_type_ref_from_type_node(&arg, src, source_symbol_index, refs);
                        }
                        _ => {}
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // Explicit type wrappers that we need to recursively process.
            "type" | "non_nullable_type" | "parenthesized_type" | "function_type" => {
                extract_type_ref_from_type_node(&child, src, source_symbol_index, refs);
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `"Hello ${expr}"` — recurse into interpolated expressions.
            "string_literal" => {
                let mut sc = child.walk();
                for seg in child.children(&mut sc) {
                    if seg.kind() == "interpolation" {
                        extract_calls_from_body(&seg, src, source_symbol_index, refs);
                    }
                }
            }
            // Annotations inside function bodies / property initializers.
            // e.g. `@Suppress("UNCHECKED_CAST")` inside a function body.
            "annotation" | "file_annotation" => {
                super::decorators::emit_annotation_ref_pub(&child, src, source_symbol_index, refs);
            }
            // property_declaration appearing in function bodies (local properties).
            // Extract the declared type as a TypeRef.
            "property_declaration" => {
                // Walk the property to find the type annotation (user_type, nullable_type, etc.)
                let mut pc = child.walk();
                for inner in child.children(&mut pc) {
                    match inner.kind() {
                        "variable_declaration" => {
                            let mut vc = inner.walk();
                            for type_child in inner.children(&mut vc) {
                                match type_child.kind() {
                                    "user_type" | "nullable_type" | "function_type"
                                    | "non_nullable_type" | "parenthesized_type" | "type" => {
                                        extract_type_ref_from_type_node(&type_child, src, source_symbol_index, refs);
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            _ => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Emit a TypeRef edge for a Kotlin `type` or `user_type` node.
pub(super) fn extract_type_ref_from_type_node(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = kotlin_type_name(node, src);
    if !name.is_empty() {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            col: 0,
            module: None,
            chain: None,
            byte_offset: node.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
});
    }
}

/// Extract the simple name from a Kotlin type node.
pub(super) fn kotlin_type_name(node: &Node, src: &[u8]) -> String {
    match node.kind() {
        "user_type" => {
            // Kotlin-ng 1.1: user_type → identifier (direct child, not simple_user_type)
            // Older Kotlin grammar: user_type → simple_user_type+
            let mut last = String::new();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "simple_user_type" {
                    let mut ic = child.walk();
                    for inner in child.children(&mut ic) {
                        if inner.kind() == "simple_identifier" || inner.kind() == "identifier" {
                            last = node_text(inner, src);
                            break;
                        }
                    }
                } else if child.kind() == "simple_identifier" || child.kind() == "identifier" {
                    last = node_text(child, src);
                } else if child.kind() == "type_identifier" {
                    last = node_text(child, src);
                }
            }
            last
        }
        "type" | "nullable_type" | "non_nullable_type" | "parenthesized_type" | "function_type" => {
            // Recurse into the inner type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let name = kotlin_type_name(&child, src);
                if !name.is_empty() {
                    return name;
                }
            }
            String::new()
        }
        "simple_identifier" | "identifier" | "type_identifier" => node_text(*node, src),
        _ => String::new(),
    }
}


/// Build a structured member access chain from a Kotlin CST node.
pub(super) fn build_chain(node: &Node, src: &[u8]) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: &Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "simple_identifier" | "identifier" => {
            let name = node_text(*node, src);
            segments.push(ChainSegment {
                name,
                node_kind: "simple_identifier".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                is_call: false,
                type_arg_ids: Vec::new(),
});
            Some(())
        }

        "this_expression" => {
            segments.push(ChainSegment {
                name: "this".to_string(),
                node_kind: "this_expression".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                is_call: false,
                type_arg_ids: Vec::new(),
});
            Some(())
        }

        "super_expression" => {
            segments.push(ChainSegment {
                name: "super".to_string(),
                node_kind: "super_expression".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                            declared_type_id: None,
                is_call: false,
                type_arg_ids: Vec::new(),
});
            Some(())
        }

        "navigation_expression" => {
            // In tree-sitter-kotlin-ng, navigation_expression children are:
            //   named_child[0]: receiver (navigation_expression | identifier | ...)
            //   unnamed: "."
            //   named_child[1]: member name (identifier | simple_identifier | navigation_suffix)
            //
            // The grammar does NOT always wrap the member in `navigation_suffix` —
            // the kotlin-ng 1.1 grammar uses plain `identifier` children directly.
            // We must handle both forms.
            let named_children: Vec<_> = {
                let mut cursor = node.walk();
                node.children(&mut cursor).filter(|c| c.is_named()).collect()
            };
            if named_children.is_empty() {
                return None;
            }
            // First named child is the receiver.
            build_chain_inner(&named_children[0], src, segments)?;
            // Remaining named children are member access segments.
            for member in named_children.iter().skip(1) {
                match member.kind() {
                    "navigation_suffix" => {
                        // Older grammar variant: navigation_suffix wraps the member name.
                        let mut nc = member.walk();
                        for inner in member.children(&mut nc) {
                            if inner.kind() == "simple_identifier" || inner.kind() == "identifier" {
                                segments.push(ChainSegment {
                                    name: node_text(inner, src),
                                    node_kind: "navigation_suffix".to_string(),
                                    kind: SegmentKind::Property,
                                    declared_type: None,
                                    type_args: vec![],
                                    optional_chaining: false,
                                    byte_offset: 0,
                                                                    declared_type_id: None,
                                    is_call: false,
                                    type_arg_ids: Vec::new(),
});
                                break;
                            }
                        }
                    }
                    "simple_identifier" | "identifier" => {
                        // kotlin-ng 1.1 variant: plain identifier as the member name.
                        segments.push(ChainSegment {
                            name: node_text(*member, src),
                            node_kind: "navigation_suffix".to_string(),
                            kind: SegmentKind::Property,
                            declared_type: None,
                            type_args: vec![],
                            optional_chaining: false,
                            byte_offset: 0,
                                                    declared_type_id: None,
                            is_call: false,
                            type_arg_ids: Vec::new(),
});
                    }
                    _ => {} // type_arguments, etc. — skip
                }
            }
            Some(())
        }

        "call_expression" => {
            // Mark the invoked segment so the walker yields the function's return
            // type rather than the function value (closure call-yield).
            let callee = node.named_child(0)?;
            build_chain_inner(&callee, src, segments)?;
            if let Some(last) = segments.last_mut() {
                last.is_call = true;
            }
            Some(())
        }

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Argument-only recursion helpers
//
// These replace the unconditional `extract_calls_from_body(&child, ...)` calls
// in the `call_expression` and `navigation_expression` arms.  Instead of
// recursing into the full node (which would re-enter every nested
// navigation_expression and emit each chain level's last segment as a Calls
// ref), we only recurse into the nodes that carry *arguments*:
//   - value_arguments       — normal argument lists  `foo(a, b)`
//   - annotated_lambda      — trailing lambda         `foo { ... }`
//   - lambda_literal        — bare lambda inside args
//   - function_literal      — grammar variant of lambda
// ---------------------------------------------------------------------------

/// Recurse into the argument nodes of a `call_expression` only.
///
/// Skips the first named child (the callee / chain), which was already handled
/// by `build_chain`.  Only processes value_arguments and trailing lambdas so
/// that calls *inside* argument expressions are captured without re-emitting
/// the callee chain segments.
fn extract_call_arguments(
    call_node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = call_node.walk();
    // Skip the first named child (callee) — process everything else.
    let mut first_named_skipped = false;
    for child in call_node.children(&mut cursor) {
        if child.is_named() && !first_named_skipped {
            first_named_skipped = true;
            continue; // this is the callee — already handled by build_chain
        }
        match child.kind() {
            "value_arguments" | "annotated_lambda" | "lambda_literal"
            | "function_literal" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // Named argument labels (`modifier =` in `foo(modifier = x)`) are
            // simple_identifier nodes that must not be emitted as refs.
            "simple_identifier" | "identifier" => {}
            _ => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Recurse into the argument-bearing children of a `navigation_expression`
/// only.  The chain itself (all nested `navigation_expression` receivers, their
/// inner `call_expression` wrappers, and navigation suffixes) has already been
/// consumed by `build_chain`; recursing into those would re-emit every
/// sub-chain segment as a spurious Calls ref.
fn extract_nav_arguments(
    nav_node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = nav_node.walk();
    for child in nav_node.children(&mut cursor) {
        match child.kind() {
            // Receiver chain, call_expressions that ARE the receiver (e.g. the
            // grammar may wrap the receiver in a call_expression node), and
            // navigation suffixes — skip entirely.  All of these are fully
            // captured by build_chain.
            "navigation_expression" | "call_expression" | "navigation_suffix"
            | "simple_identifier" | "identifier" | "this_expression" | "super_expression" => {}
            // Argument nodes — recurse to capture calls inside arguments.
            "value_arguments" | "annotated_lambda" | "lambda_literal"
            | "function_literal" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            _ => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
    }
}
