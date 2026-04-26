// =============================================================================
// csharp/types.rs  —  Type reference and annotation extraction
// =============================================================================

use super::helpers::{is_builtin_type, node_text};
use crate::parser::scope_tree;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

pub(super) fn looks_like_interface(name: &str) -> bool {
    let mut chars = name.chars();
    matches!((chars.next(), chars.next()), (Some('I'), Some(c)) if c.is_uppercase())
}

pub(super) fn simple_type_name(node: Node, src: &[u8]) -> String {
    if node.kind() == "generic_name" {
        let children: Vec<Node> = {
            let mut cursor = node.walk();
            node.children(&mut cursor).collect()
        };
        if let Some(id) = children.iter().find(|c| c.kind() == "identifier") {
            return node_text(*id, src);
        }
    }
    if node.kind() == "qualified_name" {
        let full = node_text(node, src);
        return full.rsplit('.').next().unwrap_or(&full).to_string();
    }
    node_text(node, src)
}

pub(super) fn extract_base_types(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut outer = node.walk();
    for child in node.children(&mut outer) {
        if child.kind() == "base_list" {
            let mut first_concrete = true;
            let mut cursor = child.walk();
            for base in child.children(&mut cursor) {
                match base.kind() {
                    "identifier" | "generic_name" | "qualified_name" => {
                        let name = simple_type_name(base, src);
                        let kind = if looks_like_interface(&name) {
                            EdgeKind::Implements
                        } else if first_concrete {
                            first_concrete = false;
                            EdgeKind::Inherits
                        } else {
                            EdgeKind::TypeRef
                        };
                        if looks_like_interface(&name) {
                            // Don't flip first_concrete for interfaces.
                        }
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: name,
                            kind,
                            line: base.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
});
                        // Also extract TypeRefs from generic type arguments in base types.
                        // e.g. `class Repo : BaseRepository<User>` → also emit TypeRef to User.
                        if base.kind() == "generic_name" {
                            let mut bc = base.walk();
                            for b_child in base.children(&mut bc) {
                                if b_child.kind() == "type_argument_list" {
                                    let mut tc = b_child.walk();
                                    for arg in b_child.children(&mut tc) {
                                        extract_type_refs_from_type_node(arg, src, source_idx, refs);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Walk a type node (which may be a generic, nullable, array, etc.) and emit a
/// `TypeRef` edge for every non-builtin named type found inside it.
///
/// Handles:
/// - `identifier`         — e.g. `Category`
/// - `generic_name`       — e.g. `ActionResult<Category>` → emits `ActionResult` skipped
///                          (builtin) but recurses into type_argument_list → `Category`
/// - `nullable_type`      — e.g. `Category?`
/// - `array_type`         — e.g. `Category[]`
/// - `qualified_name`     — e.g. `Foo.Bar` → simple name `Bar`
/// - `type_argument_list` — children are the generic arguments
pub(super) fn extract_type_refs_from_type_node(
    type_node: Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match type_node.kind() {
        "identifier" => {
            let name = node_text(type_node, src);
            if !name.is_empty() && !is_builtin_type(&name) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: type_node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
            }
        }
        "qualified_name" => {
            let full = node_text(type_node, src);
            let simple = full.rsplit('.').next().unwrap_or(&full).to_string();
            if !simple.is_empty() && !is_builtin_type(&simple) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: simple,
                    kind: EdgeKind::TypeRef,
                    line: type_node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
            }
        }
        "generic_name" => {
            // e.g. `ActionResult<Category>` — recurse into the identifier (the
            // outer type name) and the type_argument_list (the inner types).
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                match child.kind() {
                    "identifier" => {
                        let name = node_text(child, src);
                        if !name.is_empty() && !is_builtin_type(&name) {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::TypeRef,
                                line: child.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                                                            namespace_segments: Vec::new(),
});
                        }
                    }
                    "type_argument_list" => {
                        // Recurse into each type argument.
                        let mut ac = child.walk();
                        for arg in child.children(&mut ac) {
                            extract_type_refs_from_type_node(arg, src, source_symbol_index, refs);
                        }
                    }
                    _ => {}
                }
            }
        }
        "nullable_type" => {
            // e.g. `Category?` — the inner type is the first named child.
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                extract_type_refs_from_type_node(child, src, source_symbol_index, refs);
            }
        }
        "array_type" => {
            // e.g. `Category[]` — the element type is a named child.
            if let Some(elem) = type_node.child_by_field_name("type") {
                extract_type_refs_from_type_node(elem, src, source_symbol_index, refs);
            }
        }
        "predefined_type" => {
            // e.g. `int`, `string`, `bool` — always builtins, skip.
        }
        _ => {
            // For any other wrapper node (e.g. `ref_type`, tuple types) recurse
            // into children so we don't miss nested named types.
            let mut cursor = type_node.walk();
            for child in type_node.children(&mut cursor) {
                extract_type_refs_from_type_node(child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Extract type refs from all parameters of a parameter_list node.
pub(super) fn extract_type_refs_from_params(
    params_node: Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        if child.kind() == "parameter" {
            if let Some(type_node) = child.child_by_field_name("type") {
                extract_type_refs_from_type_node(type_node, src, source_symbol_index, refs);
            }
        }
    }
}

/// Extract typed parameters from a C# `parameter_list` node as Property symbols
/// scoped to the enclosing method or constructor.
///
/// For `void Process(UserRepository repo, int id)`, creates:
///   Symbol: `Namespace.Class.Process.repo` (kind=Property)
///   TypeRef: `Namespace.Class.Process.repo → UserRepository`
///
/// Skips parameters with predefined/builtin types and parameters without names.
///
/// C# `parameter` structure:
///   `attribute_list*`, optional `_parameter_type_with_modifiers` (has `type` field),
///   `name` field (identifier), optional default value
pub(super) fn extract_csharp_typed_params_as_symbols(
    params_node: Node,
    src: &[u8],
    scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    // Find the method/constructor scope — params are qualified under it.
    let method_scope = if params_node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, params_node.start_byte())
    } else {
        None
    };

    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        if child.kind() != "parameter" {
            continue;
        }

        let name_node = match child.child_by_field_name("name") {
            Some(n) => n,
            None => continue,
        };
        let name = node_text(name_node, src);
        if name.is_empty() {
            continue;
        }

        let type_node = match child.child_by_field_name("type") {
            Some(t) => t,
            None => continue,
        };

        // Skip predefined (builtin) types — they don't reference user symbols.
        if type_node.kind() == "predefined_type" {
            continue;
        }

        // Extract a simple type name for the TypeRef target.
        let type_name = csharp_param_type_name(type_node, src);
        if type_name.is_empty() || is_builtin_type(&type_name) {
            continue;
        }

        let qualified_name = scope_tree::qualify(&name, method_scope);
        let scope_path = scope_tree::scope_path(method_scope);

        let param_idx = symbols.len();
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name,
            kind: SymbolKind::Property,
            visibility: None,
            start_line: child.start_position().row as u32,
            end_line: child.end_position().row as u32,
            start_col: child.start_position().column as u32,
            end_col: child.end_position().column as u32,
            signature: Some(format!("{type_name} {name}")),
            doc_comment: None,
            scope_path,
            parent_index,
        });

        // Emit a TypeRef from the param symbol to its type.
        extract_type_refs_from_type_node(type_node, src, param_idx, refs);
    }
}

/// Extract a simple display name from a C# type node for use in signatures.
pub(super) fn csharp_param_type_name(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" => node_text(node, src),
        "qualified_name" => {
            let full = node_text(node, src);
            full.rsplit('.').next().unwrap_or(&full).to_string()
        }
        "generic_name" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    return node_text(child, src);
                }
            }
            String::new()
        }
        "nullable_type" => {
            // `Foo?` — extract inner type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let name = csharp_param_type_name(child, src);
                if !name.is_empty() {
                    return name;
                }
            }
            String::new()
        }
        "array_type" => {
            node.child_by_field_name("type")
                .map(|t| csharp_param_type_name(t, src))
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}
