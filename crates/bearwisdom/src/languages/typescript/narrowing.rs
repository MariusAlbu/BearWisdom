// =============================================================================
// typescript/narrowing.rs  —  Type narrowing extraction
//
// Extracts TypeRef edges from type-narrowing constructs:
//   - `x instanceof Foo`  →  TypeRef to Foo
//   - `(user as Admin).doStuff()`  →  TypeRef to Admin (as_expression in call chain)
//
// We don't do control-flow analysis — we just put the type into the graph.
// =============================================================================

use super::helpers::node_text;
use super::symbols::extract_type_ref_from_as_expression;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Walk `node` recursively and emit a TypeRef for every `instanceof` binary
/// expression and every `as_expression` that appears outside a variable
/// declarator (those are already handled in `push_variable_decl`).
pub(super) fn extract_narrowing_refs(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "binary_expression" => {
                // `x instanceof Foo`
                // binary_expression has operator field; tree-sitter-typescript uses
                // a bare `instanceof` keyword child between left and right.
                if contains_instanceof(&child) {
                    if let Some(right) = child.child_by_field_name("right") {
                        emit_type_ref_for_type_node(&right, src, source_symbol_index, refs);
                    }
                }
                // Still recurse — nested binary expressions are possible.
                extract_narrowing_refs(&child, src, source_symbol_index, refs);
            }

            "as_expression" => {
                // `(user as Admin).doAdminStuff()` — as_expression anywhere in an
                // expression context (not inside a variable_declarator, which is
                // handled by push_variable_decl / extract_type_ref_from_as_expression).
                // We emit here unconditionally; duplicates are harmless (the graph
                // deduplicates on insert) and the variable_declarator path uses the
                // same helper.
                extract_type_ref_from_as_expression(&child, src, source_symbol_index, refs);
                // Don't recurse further into the as_expression — nothing more to find.
            }

            _ => {
                extract_narrowing_refs(&child, src, source_symbol_index, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return true if `node` (a `binary_expression`) contains an `instanceof` keyword.
fn contains_instanceof(node: &Node) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "instanceof" {
            return true;
        }
    }
    false
}

/// Emit a TypeRef for the right-hand side of `instanceof Foo`.
///
/// Handles:
///   - plain `type_identifier` / `identifier`  →  `Foo`
///   - member_expression                         →  `pkg.Foo` (dotted)
fn emit_type_ref_for_type_node(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let type_name = match node.kind() {
        "type_identifier" | "identifier" => node_text(*node, src),
        "member_expression" => node_text(*node, src),
        _ => return,
    };
    if type_name.is_empty() {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: type_name,
        kind: EdgeKind::TypeRef,
        line: node.start_position().row as u32,
        module: None,
        chain: None,
        byte_offset: 0,
            namespace_segments: Vec::new(),
});
}
