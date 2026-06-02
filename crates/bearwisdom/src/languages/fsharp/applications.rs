//! F# application_expression and dot_expression ref collection.
//!
//! Walks call sites in function bodies and emits `Calls` refs.

use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

use super::extract::{is_keyword, node_text};

// ---------------------------------------------------------------------------
// Collect application_expression calls and dot_expression member accesses
// ---------------------------------------------------------------------------

/// Walk the leftmost spine of nested application_expressions to find the callee name.
///
/// `f x y` → application_expression(application_expression(f, x), y)
/// The leaf callee is the first child that is NOT application_expression.
fn extract_application_callee(node: &Node, src: &str) -> String {
    let mut current = *node;
    loop {
        if let Some(first) = current.child(0) {
            match first.kind() {
                "application_expression" => {
                    current = first;
                }
                "long_identifier_or_op" | "identifier" => {
                    return node_text(&first, src).to_string();
                }
                "dot_expression" => {
                    // e.g. `obj.Method arg` — the callee is the dot member
                    return extract_dot_member(&first, src).unwrap_or_default();
                }
                "paren_expression" | "begin_end_expression" => {
                    // e.g. `(fun x -> x) arg` — anonymous application
                    return String::new();
                }
                "infix_expression" | "ce_expression" => {
                    // e.g. `route >=> text` or `async { ... }` — compound expression,
                    // not a simple callee. The individual function refs inside will be
                    // collected by collect_applications recursing into children.
                    return String::new();
                }
                _ => {
                    // Only return text for leaf nodes (operators, keywords).
                    // Complex nodes (with children) are expressions that shouldn't
                    // be flattened into a single function name.
                    if first.child_count() == 0 {
                        let t = node_text(&first, src).to_string();
                        return t;
                    }
                    return String::new();
                }
            }
        } else {
            break;
        }
    }
    String::new()
}

pub(super) fn collect_applications(
    node: &Node,
    src: &str,
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "application_expression" => {
                // Extract the callee name: walk the leftmost spine of nested
                // application_expressions to find the actual function identifier.
                // `f x y` parses as application_expression(application_expression(f, x), y)
                // so we must recurse left to find `f`.
                let name = extract_application_callee(&child, src);
                if !name.is_empty() && !is_keyword(&name) {
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                        source_symbol_index: source_idx,
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
            "dot_expression" => {
                // dot_expression: `expr.member` — emit a Calls ref for the member name.
                // Structure: dot_expression → [expr, ".", long_identifier_or_op | identifier]
                // We want the last long_identifier_or_op or identifier child (the member name).
                if let Some(member) = extract_dot_member(&child, src) {
                    if !member.is_empty() && !is_keyword(&member) {
                        refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                            source_symbol_index: source_idx,
                            target_name: member,
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
            }
            _ => {}
        }
        collect_applications(&child, src, source_idx, refs);
    }
}

/// Extract the member name from a `dot_expression` node.
///
/// Grammar: `dot_expression = expr "." long_identifier_or_op`
/// The member name is in the last `long_identifier_or_op` or `identifier` child.
fn extract_dot_member(node: &Node, src: &str) -> Option<String> {
    let mut last_ident: Option<String> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "long_identifier_or_op" | "identifier" => {
                let t = node_text(&child, src).to_string();
                if !t.is_empty() {
                    last_ident = Some(t);
                }
            }
            _ => {}
        }
    }
    last_ident
}

