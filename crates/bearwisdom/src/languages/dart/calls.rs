// =============================================================================
// dart/calls.rs  —  Call extraction and member chain builder for Dart
// =============================================================================

use super::helpers::node_text;
use crate::types::{CallArg, ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

#[cfg(test)]
#[path = "calls_tests.rs"]
mod tests;

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
    let Some(args) = args_node else { return Vec::new() };
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
            CallArg::Await { expr: Box::new(inner) }
        }

        // `...expr` / `...?expr` — the operand is the `value` field.
        "spread_element" => {
            let inner = node
                .child_by_field_name("value")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Spread { expr: Box::new(inner) }
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
        // positional parameter names so the chain walker can type them from the
        // higher-order method's callback-parameter signature.
        "function_expression" => CallArg::Lambda { params: dart_lambda_param_names(node, src) },

        _ => CallArg::Other,
    }
}

/// Collect the positional parameter identifier names of a Dart
/// `function_expression` argument. The `parameters` field is a
/// `formal_parameter_list` of `formal_parameter` nodes wrapping an
/// `identifier`. A parameter without a plain identifier yields an empty slot so
/// positions stay aligned with the callback signature.
fn dart_lambda_param_names(node: &Node, src: &str) -> Vec<String> {
    let Some(params) = node.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut cursor = params.walk();
    params
        .named_children(&mut cursor)
        .filter(|p| p.kind() == "formal_parameter")
        .map(|p| dart_first_identifier(&p, src))
        .collect()
}

/// The first `identifier` child of a Dart `formal_parameter`, or an empty
/// string when the binding is not a plain identifier (so positions stay
/// aligned with the callback signature).
fn dart_first_identifier(node: &Node, src: &str) -> String {
    let mut i = 0;
    while let Some(child) = node.named_child(i) {
        if child.kind() == "identifier" {
            return node_text(child, src);
        }
        i += 1;
    }
    String::new()
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

/// Emit a TypeRef for a Dart `type_identifier` node.
pub(super) fn emit_dart_type_ref(
    type_node: Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = match type_node.kind() {
        "type_identifier" | "identifier" => node_text(type_node, src),
        _ => {
            // Walk for type_identifier inside type_cast, catch_clause, etc.
            let mut found = String::new();
            let mut cursor = type_node.walk();
            for child in type_node.named_children(&mut cursor) {
                if child.kind() == "type_identifier" || child.kind() == "identifier" {
                    found = node_text(child, src);
                    break;
                }
            }
            found
        }
    };
    if !name.is_empty() {
        refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::TypeRef,
            line: type_node.start_position().row as u32,
            col: 0,
            module: None,
            chain: None,
            byte_offset: type_node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    }
}

pub(super) fn extract_dart_calls(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Type references in type annotations, variable declarations, etc.
            // These appear throughout function bodies and class members.
            "type_identifier" => {
                emit_dart_type_ref(child, src, source_symbol_index, refs);
                // Do not recurse — type_identifier is a leaf.
            }

            // Generic type arguments: `List<MyType>`, `Map<String, MyModel>`.
            // type_arguments → type_argument_list → type_not_void (type_identifier, ...)
            "type_arguments" => {
                extract_type_arguments_refs(&child, src, source_symbol_index, refs);
            }

            // `x is MyType` — emit TypeRef for the test type.
            "type_test_expression" | "is_expression" => {
                extract_type_test_refs(&child, src, source_symbol_index, refs);
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // `const Foo(...)` — emit TypeRef/Calls for the constructed type.
            "const_object_expression" => {
                extract_const_object_refs(&child, src, source_symbol_index, refs);
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // Legacy node names (kept for compatibility with older grammars or future use)
            "invocation_expression" | "function_invocation" => {
                let callee_node_opt = child
                    .child_by_field_name("function")
                    .or_else(|| child.child_by_field_name("name"));

                if let Some(callee_node) = callee_node_opt {
                    let chain = build_chain(callee_node, src);
                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| dart_callee_name(callee_node, src));

                    crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &callee_node, refs);
                    if !target_name.is_empty() {
                        let call_args = extract_dart_call_args(&child, src);
                        refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                            source_symbol_index,
                            target_name,
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
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // Dart grammar 0.1: function calls are `postfix_expression` with selector(s).
            // `bar()` → postfix_expression [identifier("bar"), selector(argument_part(arguments))]
            // `obj.bar()` → postfix_expression [identifier("obj"), selector(unconditional_assignable_selector), selector(argument_part)]
            "postfix_expression" => {
                extract_postfix_call(&child, src, source_symbol_index, refs);
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // Alternative Dart call representation:
            // Dart grammar 0.1 often parses calls without a `postfix_expression` wrapper.
            // Instead, the `identifier` and `selector(argument_part(...))` appear as direct
            // siblings inside their container node.  This occurs in:
            //   expression_statement  — `bar();`
            //   initialized_variable_definition — `var d = Dog();`
            //   return_statement — `return f();`
            // Handle all of these uniformly.
            "expression_statement" | "initialized_variable_definition" | "return_statement" => {
                extract_inline_call_from_statement(&child, src, source_symbol_index, refs);
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // `new Dog(args)` — emit Calls edge to the constructed type.
            "new_expression" => {
                extract_new_expression_ref(&child, src, source_symbol_index, refs);
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // `Dog()` — implicit constructor invocation (no `new` keyword).
            // `constructor_invocation` has `type` field (type_identifier) + `arguments`.
            "constructor_invocation" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    let name = match type_node.kind() {
                        "type_identifier" | "identifier" => node_text(type_node, src),
                        _ => {
                            let mut found = String::new();
                            let mut c = type_node.walk();
                            for inner in type_node.named_children(&mut c) {
                                if inner.kind() == "type_identifier" || inner.kind() == "identifier" {
                                    found = node_text(inner, src);
                                    break;
                                }
                            }
                            found
                        }
                    };
                    if !name.is_empty() {
                        refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: child.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain: None,
                            byte_offset: child.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
                        });
                    }
                }
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // `expr as Type` — emit TypeRef for the cast type.
            // Dart grammar 0.1 structure:
            //   type_cast_expression → [..., type_cast]
            //   type_cast            → ["as", type_identifier | function_type | ...]
            "type_cast_expression" => {
                // Find the `type_cast` child which holds the target type.
                let mut tc = child.walk();
                let mut emitted = false;
                for inner in child.named_children(&mut tc) {
                    if inner.kind() == "type_cast" {
                        // Walk type_cast for type_identifier.
                        let mut ic = inner.walk();
                        for grandchild in inner.named_children(&mut ic) {
                            if grandchild.kind() == "type_identifier" || grandchild.kind() == "identifier" {
                                emit_dart_type_ref(grandchild, src, source_symbol_index, refs);
                                emitted = true;
                                break;
                            }
                        }
                        break;
                    }
                }
                // Fallback: direct type_identifier in children
                if !emitted {
                    let mut tc2 = child.walk();
                    for inner in child.named_children(&mut tc2) {
                        if inner.kind() == "type_identifier" || inner.kind() == "identifier" {
                            emit_dart_type_ref(inner, src, source_symbol_index, refs);
                            break;
                        }
                    }
                }
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // `catch (e SpecificException)` — emit TypeRef for the exception type.
            "on_part" => {
                let mut oc = child.walk();
                for inner in child.named_children(&mut oc) {
                    if inner.kind() == "type_identifier" || inner.kind() == "identifier" {
                        emit_dart_type_ref(inner, src, source_symbol_index, refs);
                        break;
                    }
                }
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }

            // String interpolation
            "string_literal_double_quotes"
            | "string_literal_single_quotes"
            | "string_literal_double_quotes_multiple"
            | "string_literal_single_quotes_multiple" => {
                let mut sc = child.walk();
                for seg in child.named_children(&mut sc) {
                    if seg.kind() == "template_substitution" {
                        extract_dart_calls(&seg, src, source_symbol_index, refs);
                    }
                }
            }

            _ => {
                extract_dart_calls(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Extract a Calls ref from a `postfix_expression` that has an argument selector
/// (i.e. is an actual function/method invocation, not just a property access).
///
/// Dart grammar 0.1 structure:
///   `bar()`        → postfix_expression [ assignable_expression(identifier("bar")),
///                                         selector(argument_part(arguments)) ]
///   `obj.bar()`   → postfix_expression [ assignable_expression(identifier("obj")),
///                                         selector(unconditional_assignable_selector(".",identifier("bar"))),
///                                         selector(argument_part(arguments)) ]
fn extract_postfix_call(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Collect all direct children upfront to avoid borrow conflicts.
    let children: Vec<tree_sitter::Node> = {
        let mut c = node.walk();
        node.children(&mut c).collect()
    };

    // Check if any selector child contains an argument_part/arguments (= a function call).
    let has_call_selector = children.iter().any(|child| {
        if child.kind() == "selector" {
            let grandchildren: Vec<_> = {
                let mut sc = child.walk();
                child.children(&mut sc).collect::<Vec<_>>()
            };
            grandchildren.iter().any(|s| s.kind() == "argument_part" || s.kind() == "arguments")
        } else {
            false
        }
    });

    if !has_call_selector {
        return;
    }

    // Find the callee: last member name from non-argument selectors, or base identifier.
    let mut last_member: Option<String> = None;
    let mut callee_from_base: Option<String> = None;

    if let Some(base) = children.first() {
        // The base is typically `assignable_expression` wrapping an identifier.
        callee_from_base = ident_from_assignable(*base, src);
    }

    for child in children.iter().skip(1) {
        if child.kind() == "selector" {
            let selector_children: Vec<_> = {
                let mut sc = child.walk();
                child.children(&mut sc).collect()
            };
            for s in &selector_children {
                match s.kind() {
                    "unconditional_assignable_selector" | "conditional_assignable_selector" => {
                        let sub: Vec<_> = {
                            let mut uc = s.walk();
                            s.children(&mut uc).collect()
                        };
                        for u in &sub {
                            if u.kind() == "identifier" || u.kind() == "type_identifier" {
                                last_member = Some(node_text(*u, src));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let target = last_member.or(callee_from_base).unwrap_or_default();
    if !target.is_empty() {
        refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
            source_symbol_index,
            target_name: target,
            kind: EdgeKind::Calls,
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

/// Handle the Dart grammar 0.1 pattern where a function call is represented as:
///   expression_statement [ identifier("bar"), selector(argument_part(arguments)) ]
/// instead of the expected postfix_expression wrapper.
///
/// This occurs for simple bare function calls like `bar()` and method calls like
/// `obj.method()` where the grammar places identifier + selector directly inside the
/// statement node without a postfix_expression wrapper.
/// Handle the Dart grammar 0.1 pattern where a function call is represented as:
///   container [ ..., identifier("callee"), selector(argument_part(arguments)), ... ]
/// instead of the expected postfix_expression wrapper.
///
/// Strategy: find the index of the first `selector(argument_part)` in the children list,
/// then take the last `identifier` or `type_identifier` that appears before that selector.
/// This correctly handles:
///   `bar()` → expression_statement(identifier("bar"), selector(...))
///   `var d = Dog()` → initialized_variable_definition(var, identifier("d"), =, identifier("Dog"), selector(...))
fn extract_inline_call_from_statement(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let children: Vec<tree_sitter::Node> = {
        let mut c = node.walk();
        node.children(&mut c).collect()
    };

    // Find index of first selector with argument_part (= the call site).
    let call_selector_idx = children.iter().position(|child| {
        if child.kind() == "selector" {
            let grandchildren: Vec<_> = {
                let mut sc = child.walk();
                child.children(&mut sc).collect::<Vec<_>>()
            };
            grandchildren.iter().any(|s| s.kind() == "argument_part" || s.kind() == "arguments")
        } else {
            false
        }
    });

    let call_idx = match call_selector_idx {
        Some(i) => i,
        None => return, // No function call selector found
    };

    // The callee: last identifier/type_identifier appearing before the call selector.
    // Also scan selector children for member access (obj.method()).
    let mut callee_ident: Option<String> = None;
    let mut last_member: Option<String> = None;

    // Scan children before the call selector for the last identifier.
    for child in &children[..call_idx] {
        match child.kind() {
            "identifier" | "type_identifier" => {
                callee_ident = Some(node_text(*child, src));
            }
            "assignable_expression" => {
                if let Some(name) = ident_from_assignable(*child, src) {
                    callee_ident = Some(name);
                }
            }
            "selector" => {
                // Non-argument selectors before the call selector = member access.
                let selector_children: Vec<_> = {
                    let mut sc = child.walk();
                    child.children(&mut sc).collect()
                };
                for s in &selector_children {
                    match s.kind() {
                        "unconditional_assignable_selector" | "conditional_assignable_selector" => {
                            let sub: Vec<_> = {
                                let mut uc = s.walk();
                                s.children(&mut uc).collect()
                            };
                            for u in &sub {
                                if u.kind() == "identifier" || u.kind() == "type_identifier" {
                                    last_member = Some(node_text(*u, src));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let target = last_member.or(callee_ident).unwrap_or_default();
    if !target.is_empty() {
        let call_args = extract_dart_call_args(node, src);
        refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
            source_symbol_index,
            target_name: target,
            kind: EdgeKind::Calls,
            line: node.start_position().row as u32,
            col: 0,
            module: None,
            chain: None,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args,
        });
    }
}

/// Extract the base identifier from an `assignable_expression` node (or plain identifier).
fn ident_from_assignable(node: tree_sitter::Node, src: &str) -> Option<String> {
    match node.kind() {
        "identifier" | "type_identifier" => Some(node_text(node, src)),
        "assignable_expression" => {
            // Walk named children looking for an identifier.
            let mut c = node.walk();
            for child in node.named_children(&mut c) {
                match child.kind() {
                    "identifier" | "type_identifier" => return Some(node_text(child, src)),
                    _ => {}
                }
            }
            // Fallback: first named child recursion
            let mut c2 = node.walk();
            for child in node.named_children(&mut c2) {
                if let Some(name) = ident_from_assignable(child, src) {
                    return Some(name);
                }
            }
            None
        }
        _ => None,
    }
}

/// Emit a Calls edge for `new Dog(args)`.
///
/// `new_expression` stores the type in the `type` field (a `type_identifier`)
/// and the arguments in the `arguments` field.  There are NO named children;
/// the type must be accessed via `child_by_field_name("type")`.
fn extract_new_expression_ref(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Try the `type` field first (Dart grammar 0.1).
    if let Some(type_node) = node.child_by_field_name("type") {
        let name = match type_node.kind() {
            "type_identifier" | "identifier" => node_text(type_node, src),
            _ => {
                // Walk into type_arguments → type_identifier
                let mut found = String::new();
                let mut c = type_node.walk();
                for child in type_node.named_children(&mut c) {
                    if child.kind() == "type_identifier" || child.kind() == "identifier" {
                        found = node_text(child, src);
                        break;
                    }
                }
                found
            }
        };
        if !name.is_empty() {
            refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                source_symbol_index,
                target_name: name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                col: 0,
                module: None,
                chain: None,
                byte_offset: node.start_byte() as u32,
                namespace_segments: Vec::new(),
                call_args: Vec::new(),
            });
            return;
        }
    }
    // Fallback: walk all children for type_identifier.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" || child.kind() == "identifier" {
            let name = node_text(child, src);
            if !name.is_empty() && name != "new" {
                refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::Calls,
                    line: child.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
                return;
            }
        }
    }
}

fn dart_callee_name(node: Node, src: &str) -> String {
    match node.kind() {
        "identifier" => node_text(node, src),
        "selector_expression" | "navigation_expression" => {
            if let Some(sel) = node.child_by_field_name("selector") {
                return node_text(sel, src);
            }
            let mut last = String::new();
            let mut c = node.walk();
            for n in node.children(&mut c) {
                if n.kind() == "identifier" || n.kind() == "simple_identifier" {
                    last = node_text(n, src);
                }
            }
            last
        }
        _ => {
            let t = node_text(node, src);
            t.rsplit('.').next().unwrap_or(&t).to_string()
        }
    }
}

pub(super) fn build_chain(node: Node, src: &str) -> Option<MemberChain> {
    if node.kind() == "identifier" {
        return None;
    }
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.len() < 2 {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: Node, src: &str, segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "identifier" | "simple_identifier" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: node.kind().to_string(),
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

        "this" => {
            segments.push(ChainSegment {
                name: "this".to_string(),
                node_kind: "this".to_string(),
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

        "super" => {
            segments.push(ChainSegment {
                name: "super".to_string(),
                node_kind: "super".to_string(),
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

        "selector_expression" => {
            let receiver = node
                .child_by_field_name("object")
                .or_else(|| node.named_child(0))?;
            build_chain_inner(receiver, src, segments)?;
            let member_name = node
                .child_by_field_name("selector")
                .map(|n| node_text(n, src))
                .or_else(|| {
                    let mut last: Option<String> = None;
                    let mut c = node.walk();
                    for child in node.children(&mut c) {
                        if child.kind() == "identifier" || child.kind() == "simple_identifier" {
                            last = Some(node_text(child, src));
                        }
                    }
                    last
                })?;
            segments.push(ChainSegment {
                name: member_name,
                node_kind: "selector_expression".to_string(),
                kind: SegmentKind::Property,
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
            let receiver = node
                .child_by_field_name("target")
                .or_else(|| node.named_child(0))?;
            build_chain_inner(receiver, src, segments)?;
            let mut last: Option<String> = None;
            let mut c = node.walk();
            for child in node.children(&mut c) {
                if child.kind() == "identifier" || child.kind() == "simple_identifier" {
                    last = Some(node_text(child, src));
                }
            }
            let member_name = last?;
            segments.push(ChainSegment {
                name: member_name,
                node_kind: "navigation_expression".to_string(),
                kind: SegmentKind::Property,
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

        "cascade_expression" => {
            let receiver = node.named_child(0)?;
            build_chain_inner(receiver, src, segments)
        }

        "invocation_expression" | "function_invocation" => {
            let callee = node
                .child_by_field_name("function")
                .or_else(|| node.child_by_field_name("name"))
                .or_else(|| node.named_child(0))?;
            build_chain_inner(callee, src, segments)
        }

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Type reference helpers added for coverage gap fixes
// ---------------------------------------------------------------------------

/// Emit TypeRef edges for all type_identifier nodes inside a `type_arguments`
/// node (e.g. `List<MyModel>`, `Map<String, UserDto>`).
pub(super) fn extract_type_arguments_refs(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "identifier" => {
                emit_dart_type_ref(child, src, source_symbol_index, refs);
            }
            // Recurse into nested type nodes (e.g. `Map<String, List<Foo>>`).
            "type_arguments" | "type_not_void" | "function_type" => {
                extract_type_arguments_refs(&child, src, source_symbol_index, refs);
            }
            _ => {
                extract_type_arguments_refs(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Emit TypeRef edges from a `type_test_expression` / `is_expression` node.
/// Dart: `x is MyType` — tree-sitter-dart 0.1 represents this as:
///   type_test_expression → [..., type_test]
///   type_test → ["is", type_not_void]
///   type_not_void → type_identifier | ...
pub(super) fn extract_type_test_refs(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_test" => {
                let mut tc = child.walk();
                for inner in child.children(&mut tc) {
                    match inner.kind() {
                        "type_identifier" | "identifier" => {
                            emit_dart_type_ref(inner, src, source_symbol_index, refs);
                        }
                        "type_not_void" | "type_not_void_not_function" => {
                            // Walk into type_not_void for the type_identifier.
                            let mut vc = inner.walk();
                            for vchild in inner.children(&mut vc) {
                                if vchild.kind() == "type_identifier" || vchild.kind() == "identifier" {
                                    emit_dart_type_ref(vchild, src, source_symbol_index, refs);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "type_identifier" | "identifier" => {
                emit_dart_type_ref(child, src, source_symbol_index, refs);
            }
            _ => {}
        }
    }
}

/// Emit TypeRef/Instantiates edges from a `const_object_expression` node.
/// Dart: `const Foo(...)` or `const package.Foo(...)`.
pub(super) fn extract_const_object_refs(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Walk children for type_identifier (the class being constructed).
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "identifier" => {
                let name = node_text(child, src);
                if !name.is_empty() && name != "const" {
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::Instantiates,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                    return;
                }
            }
            _ => {}
        }
    }
}
