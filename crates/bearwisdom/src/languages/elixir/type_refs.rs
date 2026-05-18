// =============================================================================
// Type reference extraction
//
// Walks `@type`, `@spec`, `@callback`, and `@behaviour` module attributes
// and emits TypeRef edges for each module mentioned. Also provides a
// project-wide scan that captures every `alias` node in the tree as a
// catch-all (deduped later in `extract`).
// =============================================================================

use super::helpers::{call_identifier, node_text};
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

/// Walk an attribute node and emit TypeRef edges for every `alias` node found
/// (module references like `GenServer.on_start`, `MyApp.User`, etc.).
pub(super) fn extract_attribute_type_refs(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "alias" {
            let name = node_text(child, src);
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: child.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
        }
        extract_attribute_type_refs(&child, src, source_symbol_index, refs);
    }
}

// ---------------------------------------------------------------------------
// Behaviour target extraction
// ---------------------------------------------------------------------------

/// Extract the module name from a `@behaviour GenServer` unary_operator node.
///
/// Actual tree-sitter structure (tree-sitter-elixir):
///   unary_operator
///     "@"            ← anonymous token
///     call
///       identifier   "behaviour"
///       arguments
///         alias      "GenServer"
///
/// The `@` operator's operand is a `call` node with callee "behaviour" and the
/// target module as its sole argument.
pub(super) fn extract_behaviour_target(node: &Node, src: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            // Verify this is @behaviour (not some other @attribute call)
            let callee = call_identifier(&child, src)?;
            if callee != "behaviour" && callee != "behavior" {
                return None;
            }
            // Extract the argument — the module being implemented.
            // Look for arguments field first, then walk children for arguments node.
            let args_node = if let Some(a) = child.child_by_field_name("arguments") {
                Some(a)
            } else {
                let mut cc = child.walk();
                let found = child.children(&mut cc).find(|c| c.kind() == "arguments");
                found
            };
            if let Some(args) = args_node {
                let mut ac = args.walk();
                for arg in args.children(&mut ac) {
                    match arg.kind() {
                        "alias" | "identifier" => return Some(node_text(arg, src)),
                        _ => {}
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Post-traversal full-tree type reference scan
// ---------------------------------------------------------------------------

/// Walk the entire CST and emit TypeRef edges for every `alias` node (module
/// references like `Enum`, `MyApp.User`) found in any context. Also walk
/// `dot` nodes to pick up the receiver module in `Module.function` calls.
///
/// This supplements the existing walker which only visits `alias` nodes that
/// appear as direct children of the nodes it explicitly handles.
///
/// No primitives to skip in Elixir — all alias nodes are module names.
pub(super) fn scan_all_type_refs(node: tree_sitter::Node<'_>, src: &str, refs: &mut Vec<ExtractedRef>) {
    scan_type_refs_inner(node, src, 0, refs);
}

fn scan_type_refs_inner(
    node: tree_sitter::Node<'_>,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "alias" => {
            let name = node_text(node, src);
            if !name.is_empty() {
                let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: simple,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: if name.contains('.') { Some(name) } else { None },
                    chain: None,
                    byte_offset: node.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
            // alias is a leaf — no children to recurse into.
        }
        "dot" => {
            // `dot` node represents `Module.function` — emit TypeRef for the receiver.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "alias" | "identifier" => {
                        let name = node_text(child, src);
                        if !name.is_empty() {
                            // Only emit TypeRef if it looks like a module (starts uppercase or contains dot).
                            let first_char = name.chars().next().unwrap_or('_');
                            if first_char.is_uppercase() || name.contains('.') {
                                let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
                                refs.push(ExtractedRef {
                                    source_symbol_index,
                                    target_name: simple,
                                    kind: EdgeKind::TypeRef,
                                    line: child.start_position().row as u32,
                                    module: if name.contains('.') { Some(name) } else { None },
                                    chain: None,
                                    byte_offset: child.start_byte() as u32,
                                                                    namespace_segments: Vec::new(),
                                                                    call_args: Vec::new(),
});
                            }
                        }
                        break; // only the receiver (first child), not the function name
                    }
                    _ => {}
                }
            }
            // Still recurse into dot children for nested dots/aliases.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                scan_type_refs_inner(child, src, source_symbol_index, refs);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                scan_type_refs_inner(child, src, source_symbol_index, refs);
            }
        }
    }
}
