// =============================================================================
// languages/pascal/refs.rs  —  reference-emitting extractors for Pascal/Delphi
//
// Handles `typeref` and `exprCall` nodes plus the dot-splitter shared with
// `decls::extract_class` for parent-class typeref resolution.
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

use super::extract::node_text;

// ---------------------------------------------------------------------------
// typeref  →  TypeRef (type usage references)
// typeref children include identifier / typerefDot / typerefPtr / typerefTpl
// We extract the leading identifier as the referenced type name.
// ---------------------------------------------------------------------------

pub(super) fn extract_typeref(
    node: Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or(0);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = node_text(child, src);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: name,
                        kind: EdgeKind::Calls,
                        line: node.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: node.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
                return; // one ref per typeref is enough
            }
            "typerefDot" => {
                // Qualified type: Unit.Type — split into qualifier + member
                let (member, qualifier) = split_dot_node(child, src);
                if !member.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: source_idx,
                        target_name: member,
                        kind: EdgeKind::Calls,
                        line: node.start_position().row as u32,
                        module: qualifier,
                        chain: None,
                        byte_offset: node.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
                return;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// exprCall  →  Calls
// ---------------------------------------------------------------------------

pub(super) fn extract_call(
    node: Node,
    src: &str,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let source_idx = parent_index.unwrap_or(0);
    // exprCall.entity is the callee.  Use the named field when available,
    // falling back to child(0) for grammars that omit the field name.
    let callee_opt = node.child_by_field_name("entity").or_else(|| node.child(0));
    if let Some(callee) = callee_opt {
        let (name, module) = resolve_call_target(callee, src);
        if !name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index: source_idx,
                target_name: name,
                kind: EdgeKind::Calls,
                line: node.start_position().row as u32,
                module,
                chain: None,
                byte_offset: node.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
});
        }
    }
}

/// Resolve a callee expression to `(target_name, module)`.
///
/// For qualified calls like `SysUtils.FreeAndNil`:
///   - `target_name` = "FreeAndNil"  (last segment)
///   - `module`      = Some("SysUtils")
///
/// For simple identifiers, `module` is `None`.
fn resolve_call_target(node: Node, src: &str) -> (String, Option<String>) {
    match node.kind() {
        "identifier" => (node_text(node, src), None),
        // exprDot / genericDot: children are identifier . identifier
        // Named children: [0] = qualifier, [1] = member
        "exprDot" | "genericDot" => split_dot_node(node, src),
        // Chained call: take the outer call's entity
        "exprCall" => {
            let inner = node.child_by_field_name("entity").or_else(|| node.child(0));
            inner.map(|n| resolve_call_target(n, src)).unwrap_or_default()
        }
        // Parenthesised expression — unwrap
        "exprParens" => {
            if let Some(inner) = node.named_child(0) {
                resolve_call_target(inner, src)
            } else {
                (String::new(), None)
            }
        }
        // Subscript / bracket access: take entity
        "exprBrackets" | "exprSubscript" => {
            let inner = node.child_by_field_name("entity").or_else(|| node.child(0));
            inner.map(|n| resolve_call_target(n, src)).unwrap_or_default()
        }
        // `inherited` keyword call: `inherited Create(...)` → use "inherited"
        "inherited" => ("inherited".to_string(), None),
        _ => {
            let t = node_text(node, src);
            if !t.is_empty() { (t, None) } else { (String::new(), None) }
        }
    }
}

/// Split an `exprDot` / `genericDot` / `typerefDot` node into `(member, Some(qualifier))`.
///
/// Grammar layout: identifier  kDot(.)  identifier
/// Named children (excluding anonymous punctuation) are the two identifier nodes.
/// named_child(0) = qualifier, named_child(1) = member.
pub(super) fn split_dot_node(node: Node, src: &str) -> (String, Option<String>) {
    let count = node.named_child_count();
    if count >= 2 {
        let qualifier = node.named_child(0).map(|n| node_text(n, src)).unwrap_or_default();
        let member    = node.named_child(count - 1).map(|n| node_text(n, src)).unwrap_or_default();
        if !member.is_empty() {
            return (member, if qualifier.is_empty() { None } else { Some(qualifier) });
        }
    }
    // Fallback: return full text as target_name with no module
    (node_text(node, src), None)
}

