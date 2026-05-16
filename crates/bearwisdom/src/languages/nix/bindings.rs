// =============================================================================
// languages/nix/bindings.rs  —  `inherit` / `inherit_from` extraction +
// binding name/value helpers
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::Node;

use super::calls::resolve_var_name;
use super::extract::{first_child_of_kind, first_identifier_text, is_expr_node, node_text};

// ---------------------------------------------------------------------------
// inherit  (inherit name1 name2;)
// ---------------------------------------------------------------------------

pub(super) fn extract_inherit(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    vis: Visibility,
) {
    // inherit has `inherited_attrs` field or identifier children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "inherited_attrs" => {
                let mut ac = child.walk();
                for attr in child.children(&mut ac) {
                    if attr.kind() == "identifier" {
                        let name = node_text(attr, src);
                        if !name.is_empty() {
                            emit_inherit_symbol(name, &attr, vis, parent_index, symbols);
                        }
                    }
                }
            }
            "identifier" => {
                // Direct identifier children (some grammar versions)
                let name = node_text(child, src);
                if !name.is_empty() && name != "inherit" {
                    emit_inherit_symbol(name, &child, vis, parent_index, symbols);
                }
            }
            _ => {}
        }
    }
}

fn emit_inherit_symbol(
    name: String,
    node: &Node,
    vis: Visibility,
    parent_index: Option<usize>,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: SymbolKind::Variable,
        visibility: Some(vis),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("inherit {}", name)),
        doc_comment: None,
        scope_path: None,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// inherit_from  (inherit (src) name1 name2;)
// ---------------------------------------------------------------------------

pub(super) fn extract_inherit_from(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    vis: Visibility,
) {
    // The source expression is a parenthesized expression containing a variable name
    let source_name = find_inherit_from_source(node, src);

    // Emit an Imports ref to the source if it's a named variable
    let dummy_source_idx = symbols.len();
    if let Some(src_name) = &source_name {
        refs.push(ExtractedRef {
            source_symbol_index: dummy_source_idx,
            target_name: src_name.clone(),
            kind: EdgeKind::Imports,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
});
    }

    // Extract the inherited attribute names
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            let name = node_text(child, src);
            if !name.is_empty() && name != "inherit" {
                emit_inherit_symbol(name, &child, vis, parent_index, symbols);
            }
        } else if child.kind() == "inherited_attrs" {
            let mut ac = child.walk();
            for attr in child.children(&mut ac) {
                if attr.kind() == "identifier" {
                    let name = node_text(attr, src);
                    if !name.is_empty() {
                        emit_inherit_symbol(name.clone(), &attr, vis, parent_index, symbols);
                    }
                }
            }
        }
    }
}

/// Find the source expression name in `inherit (src) ...`.
///
/// tree-sitter-nix 0.3: `inherit_from` has an `expression` named field that
/// holds the source attrset expression (the part in parentheses).
fn find_inherit_from_source(node: &Node, src: &str) -> Option<String> {
    // Primary: use the `expression` named field (tree-sitter-nix 0.3+)
    if let Some(expr) = node.child_by_field_name("expression") {
        if let Some(name) = resolve_var_name(expr, src) {
            return Some(name);
        }
        return first_identifier_text(&expr, src);
    }
    // Fallback: iterate children looking for parenthesized or variable expressions.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "parenthesized_expression" | "expression" => {
                if let Some(ident) = first_identifier_text(&child, src) {
                    return Some(ident);
                }
                let mut cc = child.walk();
                for inner in child.children(&mut cc) {
                    if inner.kind() == "variable_expression" || inner.kind() == "identifier" {
                        return Some(node_text(inner, src));
                    }
                }
            }
            "variable_expression" | "identifier" => {
                let t = node_text(child, src);
                if !t.is_empty() && t != "inherit" {
                    return Some(t);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Name/value helpers for bindings
// ---------------------------------------------------------------------------

/// Get the binding name from `attrpath` (may be dotted: a.b.c → "a.b.c").
pub(super) fn binding_name(node: &Node, src: &str) -> Option<String> {
    let attrpath = node
        .child_by_field_name("attrpath")
        .or_else(|| first_child_of_kind(node, "attrpath"))?;

    // Collect identifier children joined by "."
    let mut parts = Vec::new();
    let mut cursor = attrpath.walk();
    for child in attrpath.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "attr" {
            let t = node_text(child, src);
            if !t.is_empty() && t != "." {
                parts.push(t);
            }
        }
        // Also handle interpolated attrs (${...}) — skip those for now
    }

    if parts.is_empty() {
        // Fallback: first identifier in the binding
        first_identifier_text(node, src)
    } else {
        Some(parts.join("."))
    }
}

/// Get the value node from a binding (the expression after `=`).
pub(super) fn binding_value<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    node.child_by_field_name("expression")
        .or_else(|| {
            // Find the expression after `=` sign
            let mut cursor = node.walk();
            let mut past_eq = false;
            for child in node.children(&mut cursor) {
                if past_eq && is_expr_node(&child) {
                    return Some(child);
                }
                if node_is_eq_sign(&child) {
                    past_eq = true;
                }
            }
            None
        })
}

fn node_is_eq_sign(node: &Node) -> bool {
    // Anonymous `=` token
    node.kind() == "=" || (!node.is_named() && node.kind() == "=")
}

pub(super) fn is_function_expr(node: Node) -> bool {
    matches!(node.kind(), "function_expression" | "lambda")
}
