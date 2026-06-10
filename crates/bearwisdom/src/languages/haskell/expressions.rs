// =============================================================================
// languages/haskell/expressions.rs  —  Body-level expression extractors
//
// Function application (`apply`) and infix operator (`infix`) handling.
// These run inside any expression position — function bodies, where blocks,
// let bindings, top-level RHSs.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol};
use tree_sitter::Node;

use super::extract::node_text;

// ---------------------------------------------------------------------------
// apply  →  Calls edge
// ---------------------------------------------------------------------------

pub(super) fn extract_apply(
    node: &Node,
    src: &[u8],
    symbols: &[ExtractedSymbol],
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or_else(|| symbols.len().saturating_sub(1));
    // `function` field is optional in tree-sitter-haskell; fall back to first named child.
    let (fname, fmodule) = if let Some(func_node) = node.child_by_field_name("function") {
        extract_apply_target(func_node, src)
    } else {
        // Walk children to find the function expression
        let mut result = (String::new(), None);
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if !child.is_named() {
                    continue;
                }
                match child.kind() {
                    "variable"
                    | "name"
                    | "constructor"
                    | "qualified"
                    | "prefix_id"
                    | "operator"
                    | "operator_name"
                    | "apply"
                    | "parenthesized_expression" => {
                        let (t, m) = extract_apply_target(child, src);
                        if !t.is_empty() {
                            result = (t, m);
                            break;
                        }
                    }
                    "expression" => {
                        // Unwrap expression wrapper
                        for j in 0..child.child_count() {
                            if let Some(gc) = child.child(j) {
                                if gc.is_named() {
                                    let (t, m) = extract_apply_target(gc, src);
                                    if !t.is_empty() {
                                        result = (t, m);
                                        break;
                                    }
                                }
                            }
                        }
                        if !result.0.is_empty() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        result
    };
    if fname.is_empty() {
        return;
    }
    refs.push(ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: source_idx,
        target_name: fname,
        kind: EdgeKind::Calls,
        line: node.start_position().row as u32,
        col: 0,
        module: fmodule,
        chain: None,
        byte_offset: node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

/// Returns `(function_name, module_qualifier)` for the given callee node.
fn extract_apply_target(node: Node, src: &[u8]) -> (String, Option<String>) {
    match node.kind() {
        "variable" | "name" | "constructor" | "operator" | "operator_name" | "prefix_id" => {
            let raw = node_text(node, src)
                .trim_matches(|c: char| c == '(' || c == ')' || c == '`')
                .to_string();
            // `variable` nodes can carry a dotted qualifier when the grammar
            // doesn't produce a `qualified` node — e.g. `T.isPrefixOf` parsed
            // as a single token. Split at the last `.` so the resolver sees a
            // plain name with its module qualifier, matching the import alias.
            if let Some(dot) = raw.rfind('.') {
                let module_part = raw[..dot].to_string();
                let name_part = raw[dot + 1..].to_string();
                if !module_part.is_empty() && !name_part.is_empty() {
                    return (name_part, Some(module_part));
                }
            }
            (raw, None)
        }
        "qualified" => {
            // tree-sitter-haskell `qualified` has named fields `module` and `id`.
            // `module` contains a `module` node whose text is the full qualifier
            // (e.g. "Data.Map" for `Data.Map.lookup`).
            // `id` is the final function name.
            let id = node
                .child_by_field_name("id")
                .map(|n| node_text(n, src))
                .unwrap_or_else(|| {
                    let count = node.named_child_count();
                    if count > 0 {
                        node.named_child(count - 1)
                            .map(|n| node_text(n, src))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    }
                });
            // The `module` field node spans the qualifier including the trailing
            // `.` separator (e.g. "Map." or "Data.Map."). Strip the trailing dot.
            let module = node
                .child_by_field_name("module")
                .map(|n| node_text(n, src).trim_end_matches('.').to_string())
                .filter(|s| !s.is_empty());
            (id, module)
        }
        "apply" => {
            // Nested apply (curried) — recurse to find the base function
            node.child_by_field_name("function")
                .map(|n| extract_apply_target(n, src))
                .unwrap_or_default()
        }
        "parenthesized_expression" => {
            // Could be a section like `(+3)` or `(f)` — try first named child
            node.named_child(0)
                .map(|n| extract_apply_target(n, src))
                .unwrap_or_default()
        }
        _ => (String::new(), None),
    }
}

// ---------------------------------------------------------------------------
// infix  →  Calls edge
// ---------------------------------------------------------------------------

pub(super) fn extract_infix(
    node: &Node,
    src: &[u8],
    symbols: &[ExtractedSymbol],
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or_else(|| symbols.len().saturating_sub(1));
    // infix: left operator right
    // The operator field holds the infix function name (backtick or operator)
    let op_node = node.child_by_field_name("operator").or_else(|| {
        // Fallback: find the operator child — check all children (named or anonymous)
        let count = node.child_count();
        if count >= 3 {
            // Middle child (index 1 in 3-child infix: left op right)
            node.child(1)
        } else {
            // Second named child fallback
            let mut cursor = node.walk();
            let children: Vec<Node> = node.children(&mut cursor).collect();
            let named: Vec<Node> = children.into_iter().filter(|c| c.is_named()).collect();
            if named.len() >= 2 {
                Some(named[1])
            } else {
                None
            }
        }
    });

    let op_text = op_node
        .map(|n| {
            let t = node_text(n, src);
            // Strip backtick quoting from infix functions like `elem`
            t.trim_matches('`').to_string()
        })
        .unwrap_or_default();

    if op_text.is_empty() {
        return;
    }

    refs.push(ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: source_idx,
        target_name: op_text,
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
