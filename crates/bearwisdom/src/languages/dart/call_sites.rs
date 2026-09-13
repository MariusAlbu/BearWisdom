// =============================================================================
// dart/call_sites.rs  —  the Calls ref of one invocation's selector sequence
//
// The grammar spells a call as a callee identifier followed by member and
// argument selectors, either wrapped in a `postfix_expression` or laid out
// as direct siblings of the containing node. Both shapes reduce to the same
// ref: the last member (or the callee) as the target, the preceding members
// as the receiver chain, the argument list as the call's arguments.
// =============================================================================

use super::call_args::extract_dart_call_args;
use super::helpers::node_text;
use crate::types::{CallArg, ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

/// Extract a Calls ref from a `postfix_expression` that has an argument selector
/// (i.e. is an actual function/method invocation, not just a property access).
///
/// Dart grammar 0.1 structure:
///   `bar()`        → postfix_expression [ assignable_expression(identifier("bar")),
///                                         selector(argument_part(arguments)) ]
///   `obj.bar()`   → postfix_expression [ assignable_expression(identifier("obj")),
///                                         selector(unconditional_assignable_selector(".",identifier("bar"))),
///                                         selector(argument_part(arguments)) ]
pub(super) fn extract_postfix_call(
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
            grandchildren
                .iter()
                .any(|s| s.kind() == "argument_part" || s.kind() == "arguments")
        } else {
            false
        }
    });

    if !has_call_selector {
        return;
    }

    // Find the callee: last member name from non-argument selectors, or base identifier.
    // Keep every member selector as well: callback lexical capture needs the
    // receiver root, not just the selected method's display name.
    let mut members = Vec::new();
    let mut chain_supported = true;
    // The base is typically `assignable_expression` wrapping an identifier.
    let callee_from_base = children
        .first()
        .and_then(|base| ident_node_from_assignable(*base));

    for child in children.iter().skip(1) {
        match selector_member_nodes(*child) {
            Some(selector_members) => members.extend(selector_members),
            None => chain_supported = false,
        }
    }

    let target = members
        .last()
        .map(|(member, _)| node_text(*member, src))
        .or_else(|| callee_from_base.map(|base| node_text(base, src)))
        .unwrap_or_default();
    if !target.is_empty() {
        let call_args = extract_dart_call_args(node, src);
        let chain = callee_from_base
            .filter(|_| chain_supported && !members.is_empty())
            .map(|receiver| member_call_chain(receiver, &members, &call_args, src));
        refs.push(ExtractedRef {
            is_include: false,
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index,
            target_name: target,
            kind: EdgeKind::Calls,
            line: node.start_position().row as u32,
            col: 0,
            module: None,
            chain,
            byte_offset: node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args,
        });
    }
}

/// Build the receiver-to-selector chain for a Dart member call. Bare
/// calls intentionally have no chain: there is no receiver identity to attest.
fn member_call_chain(
    receiver: Node,
    members: &[(Node, bool)],
    call_args: &[CallArg],
    src: &str,
) -> MemberChain {
    let mut segments = Vec::with_capacity(members.len() + 1);
    segments.push(ChainSegment {
        name: node_text(receiver, src),
        node_kind: receiver.kind().to_string(),
        kind: SegmentKind::Identifier,
        declared_type: None,
        type_args: Vec::new(),
        optional_chaining: false,
        byte_offset: receiver.start_byte() as u32,
        declared_type_id: None,
        is_call: false,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    });
    let member_count = members.len();
    segments.extend(
        members
            .iter()
            .enumerate()
            .map(|(index, (member, optional))| {
                let is_terminal = index + 1 == member_count;
                ChainSegment {
                    name: node_text(*member, src),
                    node_kind: member.kind().to_string(),
                    kind: SegmentKind::Property,
                    declared_type: None,
                    type_args: Vec::new(),
                    optional_chaining: *optional,
                    byte_offset: member.start_byte() as u32,
                    declared_type_id: None,
                    is_call: is_terminal,
                    call_args: is_terminal.then(|| call_args.to_vec()).unwrap_or_default(),
                    type_arg_ids: Vec::new(),
                }
            }),
    );
    MemberChain { segments }
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
pub(super) fn extract_inline_call_from_statement(
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
            grandchildren
                .iter()
                .any(|s| s.kind() == "argument_part" || s.kind() == "arguments")
        } else {
            false
        }
    });

    let call_idx = match call_selector_idx {
        Some(i) => i,
        None => return, // No function call selector found
    };

    // The callee: last identifier/type_identifier appearing before the call selector.
    // Also retain member selectors so a method call carries its receiver root.
    let mut callee_ident: Option<Node> = None;
    let mut members = Vec::new();
    let mut chain_supported = true;

    // Scan children before the call selector for the last identifier.
    for child in &children[..call_idx] {
        match child.kind() {
            "identifier" | "type_identifier" => {
                callee_ident = Some(*child);
            }
            "assignable_expression" => {
                if let Some(identifier) = ident_node_from_assignable(*child) {
                    callee_ident = Some(identifier);
                }
            }
            "selector" => match selector_member_nodes(*child) {
                Some(selector_members) => members.extend(selector_members),
                None => chain_supported = false,
            },
            _ => {}
        }
    }

    let target = members
        .last()
        .map(|(member, _)| node_text(*member, src))
        .or_else(|| callee_ident.map(|identifier| node_text(identifier, src)))
        .unwrap_or_default();
    if !target.is_empty() {
        let call_args = extract_dart_call_args(node, src);
        let chain = callee_ident
            .filter(|_| chain_supported && !members.is_empty())
            .map(|receiver| member_call_chain(receiver, &members, &call_args, src));
        // The ref is positioned at the call's own head, not at the statement
        // that contains it: a binding's initializer is correlated by the byte
        // range of its value expression, which starts after `var x =`.
        let site = callee_ident.unwrap_or(*node);
        refs.push(ExtractedRef {
            is_include: false,
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index,
            target_name: target,
            kind: EdgeKind::Calls,
            line: site.start_position().row as u32,
            col: 0,
            module: None,
            chain,
            byte_offset: site.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args,
        });
    }
}

/// Extract the base identifier from an `assignable_expression` node (or plain identifier).
fn ident_node_from_assignable(node: Node) -> Option<Node> {
    match node.kind() {
        "identifier" | "type_identifier" => Some(node),
        "assignable_expression" => {
            // Walk named children looking for an identifier.
            let mut c = node.walk();
            for child in node.named_children(&mut c) {
                match child.kind() {
                    "identifier" | "type_identifier" => return Some(child),
                    _ => {}
                }
            }
            // Fallback: first named child recursion
            let mut c2 = node.walk();
            for child in node.named_children(&mut c2) {
                if let Some(identifier) = ident_node_from_assignable(child) {
                    return Some(identifier);
                }
            }
            None
        }
        _ => None,
    }
}

/// Extract member name nodes from one selector, preserving whether the grammar
/// made that navigation conditional. An unsupported selector returns `None`
/// so callers decline the full receiver chain rather than eliding a step.
fn selector_member_nodes(selector: Node) -> Option<Vec<(Node, bool)>> {
    if selector.kind() != "selector" {
        return Some(Vec::new());
    }
    let mut selector_cursor = selector.walk();
    let children: Vec<_> = selector.children(&mut selector_cursor).collect();
    if children
        .iter()
        .any(|child| matches!(child.kind(), "argument_part" | "arguments"))
    {
        return Some(Vec::new());
    }

    let mut members = Vec::new();
    for child in children {
        let optional = match child.kind() {
            "unconditional_assignable_selector" => false,
            "conditional_assignable_selector" => true,
            _ => continue,
        };
        let mut member_cursor = child.walk();
        let member = child
            .children(&mut member_cursor)
            .find(|member| matches!(member.kind(), "identifier" | "type_identifier"))?;
        members.push((member, optional));
    }
    (!members.is_empty()).then_some(members)
}
