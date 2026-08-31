// =============================================================================
// python/types.rs — type alias declarations and annotation TypeRef emission
// =============================================================================

use super::helpers::{detect_python_visibility, node_text, qualify, scope_from_prefix};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

pub(super) fn extract_type_alias_top_level(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
) {
    extract_type_alias(
        node,
        source,
        symbols,
        refs,
        parent_index,
        qualified_prefix,
        enclosing_symbol_index,
    );
}

/// Extract `type Point = tuple[int, int]` as a TypeAlias symbol.
pub(super) fn extract_type_alias(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
) {
    // tree-sitter-python uses "left" for the alias name and "right" for the
    // aliased type in `type_alias_statement` (field names differ from "name"/"value").
    let name_node = match node.child_by_field_name("left") {
        Some(n) => n,
        None => return,
    };
    // The "left" child is a "type" wrapper node; the actual identifier is inside it.
    let name = if name_node.kind() == "type" {
        name_node
            .named_child(0)
            .map(|c| node_text(&c, source))
            .unwrap_or_else(|| node_text(&name_node, source))
    } else {
        node_text(&name_node, source)
    };
    let qualified_name = qualify(&name, qualified_prefix);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::TypeAlias,
        visibility: detect_python_visibility(&name),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("type {name} = ...")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });

    // Extract TypeRef edges from the aliased type expression.
    // The "right" field holds the aliased type (also wrapped in a "type" node).
    if let Some(value) = node.child_by_field_name("right") {
        extract_type_refs_from_annotation(&value, source, idx, refs);
    }

    let _ = enclosing_symbol_index; // not used here but kept for API consistency
}

/// Walk a type annotation node and emit TypeRef edges for all identifiers found.
fn extract_type_refs_from_annotation(
    node: &Node,
    source: &str,
    symbol_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source);
            if !name.is_empty() && name != "None" {
                refs.push(ExtractedRef {
                    is_include: false,
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: symbol_idx,
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
        // `uuid.UUID` or `sqlalchemy.orm.Session` — emit a single ref for the
        // rightmost identifier with the object path as `module`, so downstream
        // classification can route via the import map. Do NOT recurse into
        // the attribute's children (which would otherwise emit a spurious
        // bare ref for every segment).
        "attribute" => {
            if let Some(attr) = node.child_by_field_name("attribute") {
                let name = node_text(&attr, source);
                if !name.is_empty() {
                    let module = node
                        .child_by_field_name("object")
                        .map(|o| node_text(&o, source))
                        .filter(|s| !s.is_empty());
                    refs.push(ExtractedRef {
                        is_include: false,
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index: symbol_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: attr.start_position().row as u32,
                        col: 0,
                        module,
                        chain: None,
                        byte_offset: attr.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    extract_type_refs_from_annotation(&child, source, symbol_idx, refs);
                }
            }
        }
    }
}
