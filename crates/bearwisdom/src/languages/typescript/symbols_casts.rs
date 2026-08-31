use super::helpers::node_text;
use super::types::extract_type_ref_from_annotation;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

/// Extract a TypeRef from `user as Admin` — the `as_expression` node.
///
/// Tree-sitter structure:
/// ```text
/// as_expression
///   identifier "user"      ← expression
///   "as"
///   type_identifier "Admin" ← asserted type
/// ```
/// We look for a `type_identifier`, `generic_type`, or `identifier` child that
/// appears after the `as` keyword.
pub(super) fn extract_type_ref_from_as_expression(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut after_as = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "as" {
            after_as = true;
            continue;
        }
        if !after_as {
            continue;
        }
        // First node after `as` is the asserted type — delegate to the full
        // annotation handler so all complex type forms (union, generic, etc.) work.
        extract_type_ref_from_annotation(&child, src, source_symbol_index, refs);
        return;
    }
}

/// Extract a TypeRef from `expr satisfies TypeName` — the `satisfies_expression` node.
///
/// Tree-sitter structure:
/// ```text
/// satisfies_expression
///   <expression>           ← the value being checked
///   "satisfies"
///   type_identifier "Config"  ← the asserted type
/// ```
pub(super) fn extract_type_ref_from_satisfies_expression(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut after_satisfies = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "satisfies" {
            after_satisfies = true;
            continue;
        }
        if !after_satisfies {
            continue;
        }
        // First node after `satisfies` is the asserted type.
        // Delegate to the full annotation handler so all complex type forms
        // (union, generic, intersection, conditional, etc.) are covered.
        extract_type_ref_from_annotation(&child, src, source_symbol_index, refs);
        return;
    }
}

/// Extract a TypeRef from `<Admin>user` — the `type_assertion` node.
///
/// Tree-sitter structure:
/// ```text
/// type_assertion
///   type_arguments
///     type_identifier "Admin"
///   identifier "user"
/// ```
pub(super) fn extract_type_ref_from_type_assertion(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let type_args = if let Some(n) = node.child_by_field_name("type_arguments") {
        n
    } else {
        // Fallback: find the first child of kind "type_arguments".
        let mut found: Option<Node> = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "type_arguments" {
                found = Some(child);
                break;
            }
        }
        match found {
            Some(n) => n,
            None => return,
        }
    };

    let mut cursor = type_args.walk();
    for child in type_args.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "identifier" => {
                let type_name = node_text(child, src);
                if !type_name.is_empty() {
                    refs.push(ExtractedRef {
                        is_include: false,
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index,
                        target_name: type_name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
                return;
            }
            "generic_type" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let type_name = node_text(name_node, src);
                    if !type_name.is_empty() {
                        refs.push(ExtractedRef {
                            is_include: false,
                            is_import_binding: false,
                            is_reexport: false,
                            source_symbol_index,
                            target_name: type_name,
                            kind: EdgeKind::TypeRef,
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
                return;
            }
            _ => {}
        }
    }
}
