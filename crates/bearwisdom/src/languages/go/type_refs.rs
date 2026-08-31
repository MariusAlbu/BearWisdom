// =============================================================================
// go/type_refs.rs  —  Type reference extraction and type-context detection
// =============================================================================

use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

/// Check if a selector_expression node is in a type context
/// (e.g., parameter type, return type, var type, cast target).
pub(super) fn is_in_type_context(node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            // Type positions in parameter declarations
            "parameter_declaration" => {
                // The type field of a parameter is in a type context
                if let Some(type_field) = parent.child_by_field_name("type") {
                    if type_field.id() == node.id() || ancestor_of(&type_field, node) {
                        return true;
                    }
                }
            }
            // Type positions in var/const declarations
            "const_spec" | "var_spec" => {
                // The type field is in a type context
                if let Some(type_field) = parent.child_by_field_name("type") {
                    if type_field.id() == node.id() || ancestor_of(&type_field, node) {
                        return true;
                    }
                }
            }
            // Type in type conversion expression
            "type_conversion_expression" => {
                if let Some(type_field) = parent.child_by_field_name("type") {
                    if type_field.id() == node.id() || ancestor_of(&type_field, node) {
                        return true;
                    }
                }
            }
            // result / return type
            "result" => return true,
            // Stop searching at function/method boundaries
            "function_declaration" | "method_declaration" => break,
            // Most other contexts are value contexts; keep searching
            _ => {}
        }
        current = parent.parent();
    }
    false
}

/// Check if `ancestor` is an ancestor of `node`.
fn ancestor_of(ancestor: &Node, node: &Node) -> bool {
    let mut current = node.parent();
    while let Some(p) = current {
        if p.id() == ancestor.id() {
            return true;
        }
        current = p.parent();
    }
    false
}

/// Extract TypeRef edges from function/method parameter types and return types.
///
/// Walks the parameter_list and result nodes, emitting TypeRef for each
/// non-builtin type found.
pub(super) fn extract_fn_signature_type_refs(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Extract param types.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "parameter_list" {
            extract_type_refs_from_param_list(&child, source, source_symbol_index, refs);
        } else if child.kind() == "result" {
            // result can be a single type or a parameter_list for multiple return types.
            if let Some(plist) = child.child_by_field_name("parameters") {
                extract_type_refs_from_param_list(&plist, source, source_symbol_index, refs);
            } else {
                // Single return type — the first named child.
                // Walk its subtree to emit TypeRef for all type_identifier nodes.
                if let Some(first) = child.named_child(0) {
                    emit_type_refs_from_type_node(&first, source, source_symbol_index, refs);
                }
            }
        }
    }
}

pub(super) fn extract_type_refs_from_param_list(
    param_list: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = param_list.walk();
    for child in param_list.children(&mut cursor) {
        if child.kind() == "parameter_declaration"
            || child.kind() == "variadic_parameter_declaration"
        {
            // Walk the type subtree to emit TypeRef for every type_identifier
            // inside the parameter type (handles maps, slices, channels, etc.).
            if let Some(type_node) = child.child_by_field_name("type") {
                emit_type_refs_from_type_node(&type_node, source, source_symbol_index, refs);
            }
        }
    }
}

/// Walk a Go type AST node and emit `TypeRef` edges for every `type_identifier`
/// and `qualified_type` that is not a builtin. Handles composite types
/// (slices, maps, channels, pointers, function types, generics) by recursing
/// into named children. A `qualified_type` (`pkg.Type`) emits the bare member
/// name as `target_name` with the package qualifier carried on `module` — the
/// same shape [`super::qualified_types::go_type_ref_target`] produces
/// everywhere else, so the resolve engine's module-qualified rungs bind it.
pub(super) fn emit_type_refs_from_type_node(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "type_identifier" | "qualified_type" => {
            if let Some((name, module)) = super::qualified_types::go_type_ref_target(node, source)
            {
                if !name.is_empty() && !super::helpers::is_go_builtin_type(&name) {
                    refs.push(ExtractedRef {
                        is_include: false,
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: node.start_position().row as u32,
                        col: 0,
                        module,
                        chain: None,
                        byte_offset: node.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
            }
        }
        _ => {
            // For all composite types, recurse into named children.
            if node.is_named() && node.child_count() > 0 {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.is_named() {
                        emit_type_refs_from_type_node(&child, source, source_symbol_index, refs);
                    }
                }
            }
        }
    }
}
