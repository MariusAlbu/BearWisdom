// =============================================================================
// csharp/symbols/members.rs  —  member declarations (methods, ctors, props, fields)
//
// Pushes Method, Constructor, Property, Accessor, Field, EventField, and
// Delegate symbols, plus type-ref helpers for method/constructor parameter
// and return types.
// =============================================================================

use super::super::helpers::{
    build_method_signature, detect_visibility, extract_doc_comment, find_child_kind, has_modifier,
    has_test_attribute, node_text,
};
use super::super::types::{extract_type_refs_from_params, extract_type_refs_from_type_node};
use crate::parser::scope_tree::{self, ScopeTree};
use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

pub(in super::super) fn push_method_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    // The method's own scope covers its body — we want the parent (the class).
    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let kind = if has_test_attribute(node, src) {
        SymbolKind::Test
    } else {
        SymbolKind::Method
    };

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: build_method_signature(node, src),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// Extract type refs from a method's return type and parameter types.
/// Called after the symbol is pushed so we know its index.
pub(in super::super) fn push_method_type_refs(
    node: &Node,
    src: &[u8],
    symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Return type is the `returns` field on method_declaration.
    if let Some(ret_node) = node.child_by_field_name("returns") {
        extract_type_refs_from_type_node(ret_node, src, symbol_index, refs);
    }
    // Parameter types.
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_type_refs_from_params(params, src, symbol_index, refs);
    }
}

pub(in super::super) fn push_constructor_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(p, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Constructor,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{name}{params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// Extract type refs from a constructor's parameter types.
pub(in super::super) fn push_constructor_type_refs(
    node: &Node,
    src: &[u8],
    symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_type_refs_from_params(params, src, symbol_index, refs);
    }
}

pub(in super::super) fn push_property_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let type_str = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Property,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{type_str} {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    // Emit a TypeRef edge for the property's declared type.
    if let Some(type_node) = node.child_by_field_name("type") {
        extract_type_refs_from_type_node(type_node, src, idx, refs);
    }
}

/// Emit a Method symbol for an `accessor_declaration` (get/set/init) inside a
/// property or indexer body.
///
/// `accessor_declaration` has no `name` field in tree-sitter-c-sharp — the
/// accessor kind is an anonymous keyword token ("get", "set", "init", "add",
/// "remove").  We scan the children for that keyword to build the name.
///
/// The resulting symbol is named `<PropertyName>.get` (or `.set`/`.init`), but
/// for simplicity we use the raw accessor keyword as the name and qualify it
/// under the enclosing property scope.
pub(in super::super) fn push_accessor_decl(
    accessor_node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    // The accessor keyword ("get", "set", "init", "add", "remove") is the
    // first non-attribute token child.
    let accessor_kind = {
        let mut cursor = accessor_node.walk();
        let mut found: Option<String> = None;
        for child in accessor_node.children(&mut cursor) {
            match child.kind() {
                "get" | "set" | "init" | "add" | "remove" => {
                    found = Some(node_text(child, src));
                    break;
                }
                _ => {}
            }
        }
        found?
    };

    // Qualify under the enclosing property scope (one level up).
    let parent_scope = if accessor_node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, accessor_node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&accessor_kind, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: accessor_kind.clone(),
        qualified_name,
        kind: SymbolKind::Method,
        visibility: detect_visibility(accessor_node, src),
        start_line: accessor_node.start_position().row as u32,
        end_line: accessor_node.end_position().row as u32,
        start_col: accessor_node.start_position().column as u32,
        end_col: accessor_node.end_position().column as u32,
        signature: Some(accessor_kind),
        doc_comment: None,
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(in super::super) fn push_field_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let _is_const = has_modifier(node, "const");
    let kind = SymbolKind::Field;
    let visibility = detect_visibility(node, src);
    let doc_comment = extract_doc_comment(node, src);

    let var_decl = match find_child_kind(node, "variable_declaration") {
        Some(v) => v,
        None => return,
    };
    let type_str = var_decl
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();

    // Grab the type node once; we'll emit a TypeRef per field declarator.
    let type_node_opt = var_decl.child_by_field_name("type");

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let scope_path = scope_tree::scope_path(parent_scope);

    let mut cursor = var_decl.walk();
    for declarator in var_decl.children(&mut cursor) {
        if declarator.kind() == "variable_declarator" {
            if let Some(name_node) = declarator.child_by_field_name("name") {
                let name = node_text(name_node, src);
                let qualified_name = scope_tree::qualify(&name, parent_scope);
                let idx = symbols.len();
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
                    qualified_name,
                    kind,
                    visibility,
                    start_line: declarator.start_position().row as u32,
                    end_line: declarator.end_position().row as u32,
                    start_col: declarator.start_position().column as u32,
                    end_col: declarator.end_position().column as u32,
                    signature: Some(format!("{type_str} {name}")),
                    doc_comment: doc_comment.clone(),
                    scope_path: scope_path.clone(),
                    parent_index,
                });
                // Emit a TypeRef for the field's declared type.
                if let Some(tn) = type_node_opt {
                    extract_type_refs_from_type_node(tn, src, idx, refs);
                }
            }
        }
    }
}

pub(in super::super) fn push_event_field_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let visibility = detect_visibility(node, src);
    let doc_comment = extract_doc_comment(node, src);

    let var_decl = match find_child_kind(node, "variable_declaration") {
        Some(v) => v,
        None => return,
    };
    let type_str = var_decl
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let scope_path = scope_tree::scope_path(parent_scope);

    let mut cursor = var_decl.walk();
    for declarator in var_decl.children(&mut cursor) {
        if declarator.kind() == "variable_declarator" {
            if let Some(name_node) = declarator.child_by_field_name("name") {
                let name = node_text(name_node, src);
                let qualified_name = scope_tree::qualify(&name, parent_scope);
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
                    qualified_name,
                    kind: SymbolKind::Event,
                    visibility,
                    start_line: node.start_position().row as u32,
                    end_line: node.end_position().row as u32,
                    start_col: node.start_position().column as u32,
                    end_col: node.end_position().column as u32,
                    signature: Some(format!("event {type_str} {name}")),
                    doc_comment: doc_comment.clone(),
                    scope_path: scope_path.clone(),
                    parent_index,
                });
            }
        }
    }
}

pub(in super::super) fn push_delegate_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(name_node, src);

    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let ret = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();
    let params = node
        .child_by_field_name("parameters")
        .map(|p| node_text(p, src))
        .unwrap_or_default();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Delegate,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("delegate {ret} {name}{params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
}
