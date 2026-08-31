//! F# type-member extraction (`method_or_prop_defn`).
//!
//! A `member this.Name …` node yields a Method or Property symbol parented to
//! the enclosing type, so member lookups on internal receivers land on a real
//! symbol row. Refs collected from the member's body attribute to the member
//! itself, and nested declarations parent under it.

use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::Node;

use super::applications::collect_applications;
use super::extract::{node_text, qualify_with_parent, scope_path_from_parent, visit};

/// Emit the member symbol for a `method_or_prop_defn`, then collect its body's
/// call refs and recurse. A defn with argument patterns is a Method; the
/// value / accessor forms are Properties. Operator members and unnameable
/// defns keep the previous behavior: body refs attribute to the enclosing
/// type and no symbol is emitted.
pub(super) fn extract_method_or_prop(
    node: &Node,
    src: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let Some(name) = member_name(node, src) else {
        let source_idx = parent_index.unwrap_or(0);
        collect_applications(node, src, source_idx, refs);
        visit(*node, src, symbols, refs, parent_index);
        return;
    };
    let kind = if node.child_by_field_name("args").is_some() {
        SymbolKind::Method
    } else {
        SymbolKind::Property
    };
    let qualified_name = qualify_with_parent(&name, parent_index, symbols);
    let scope_path = scope_path_from_parent(parent_index, symbols);
    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: 0,
        signature: Some(format!("member {}", name)),
        doc_comment: None,
        scope_path,
        parent_index,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });
    collect_applications(node, src, idx, refs);
    visit(*node, src, symbols, refs, Some(idx));
}

/// The member's declared name from the `name` field (`property_or_ident`):
/// the `method` half of an `instance.method` head, or the bare identifier.
/// Operator heads and empty names decline.
fn member_name(node: &Node, src: &str) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    let text = match name_node.child_by_field_name("method") {
        Some(method) => node_text(&method, src).to_string(),
        None => node_text(&name_node, src).to_string(),
    };
    // An `x.Member` head that arrives unfielded still names the member last.
    let leaf = text.rsplit('.').next().unwrap_or(&text).trim().to_string();
    let is_plain_identifier = {
        let mut chars = leaf.chars();
        chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
            && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '\'')
    };
    if leaf.is_empty() || !is_plain_identifier {
        return None;
    }
    Some(leaf)
}
