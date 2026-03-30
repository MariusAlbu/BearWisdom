// =============================================================================
// csharp/symbols.rs  —  Symbol pushers (one per declaration kind)
// =============================================================================

use super::helpers::{
    build_method_signature, detect_visibility, extract_doc_comment, find_child_kind, has_modifier,
    has_test_attribute, node_text,
};
use super::types::{extract_type_refs_from_params, extract_type_refs_from_type_node};
use crate::parser::scope_tree::{self, ScopeTree};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

pub(super) fn push_namespace(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);
    // Use parent scope (byte before this node), not the namespace's own scope.
    // Same pattern as push_type_decl — prevents doubled names like "App.App".
    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("namespace {name}")),
        doc_comment: None,
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_type_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    kind: SymbolKind,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    // The scope tree has an entry for this class at this byte position.
    // We find the scope CONTAINING this class (its parent), not the class scope itself.
    // The class's own scope entry has start_byte == node.start_byte().
    // `find_scope_at` returns the deepest scope covering the start byte —
    // which will be the class itself if depth > 0.
    // We want the parent scope, so we look up the position just *before* this node.
    let parent_scope = if node.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, node.start_byte() - 1)
    } else {
        None
    };
    let qualified_name = scope_tree::qualify(&name, parent_scope);
    let scope_path = scope_tree::scope_path(parent_scope);

    let keyword = match kind {
        SymbolKind::Struct => "struct",
        SymbolKind::Interface => "interface",
        _ => "class",
    };
    let type_params = node
        .child_by_field_name("type_parameters")
        .map(|tp| node_text(tp, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{keyword} {name}{type_params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

/// Extract primary constructor parameters of a record as Property symbols.
///
/// `record Point(int X, int Y)` — `X` and `Y` are synthesised as public
/// init-only properties by the compiler.  We extract them so the index
/// knows they exist (they won't appear in a body as `property_declaration`).
pub(super) fn extract_record_primary_params(
    record_node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    record_sym_idx: usize,
) {
    // The record's own scope covers its parameter list and body.
    // We find the record's qualified name from the symbol we just pushed.
    let record_qname = symbols[record_sym_idx].qualified_name.clone();

    // In tree-sitter-c-sharp, `parameter_list` is an unnamed child of
    // `record_declaration`, not a named field.  Use find_child_kind.
    let param_list = match find_child_kind(record_node, "parameter_list") {
        Some(pl) => pl,
        None => return, // record without a primary constructor parameter list
    };

    let mut cursor = param_list.walk();
    for param in param_list.children(&mut cursor) {
        if param.kind() != "parameter" {
            continue;
        }
        let name_node = match param.child_by_field_name("name") {
            Some(n) => n,
            None => continue,
        };
        let name = node_text(name_node, src);
        let type_str = param
            .child_by_field_name("type")
            .map(|t| node_text(t, src))
            .unwrap_or_default();

        let qualified_name = format!("{record_qname}.{name}");
        // Use the record's own scope entry as the parent scope.
        let parent_scope = scope_tree::find_scope_at(scope_tree, param.start_byte());
        let scope_path = Some(record_qname.clone());
        let _ = parent_scope; // scope lookup not needed — we derive scope_path directly

        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name,
            kind: SymbolKind::Property,
            visibility: Some(crate::types::Visibility::Public),
            start_line: param.start_position().row as u32,
            end_line: param.end_position().row as u32,
            start_col: param.start_position().column as u32,
            end_col: param.end_position().column as u32,
            signature: Some(format!("{type_str} {name}")),
            doc_comment: None,
            scope_path,
            parent_index: Some(record_sym_idx),
        });
    }
}

pub(super) fn push_enum_decl(
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

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: qualified_name.clone(),
        kind: SymbolKind::Enum,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("enum {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });

    // Extract enum members.
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for member in body.children(&mut cursor) {
            if member.kind() == "enum_member_declaration" {
                if let Some(mname_node) = member.child_by_field_name("name") {
                    let mname = node_text(mname_node, src);
                    let mqualified = format!("{qualified_name}.{mname}");
                    symbols.push(ExtractedSymbol {
                        name: mname,
                        qualified_name: mqualified,
                        kind: SymbolKind::EnumMember,
                        visibility: None,
                        start_line: member.start_position().row as u32,
                        end_line: member.end_position().row as u32,
                        start_col: member.start_position().column as u32,
                        end_col: member.end_position().column as u32,
                        signature: None,
                        doc_comment: extract_doc_comment(&member, src),
                        scope_path: Some(qualified_name.clone()),
                        parent_index: Some(idx),
                    });
                }
            }
        }
    }

    Some(idx)
}

pub(super) fn push_method_decl(
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
pub(super) fn push_method_type_refs(
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

pub(super) fn push_constructor_decl(
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
pub(super) fn push_constructor_type_refs(
    node: &Node,
    src: &[u8],
    symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if let Some(params) = node.child_by_field_name("parameters") {
        extract_type_refs_from_params(params, src, symbol_index, refs);
    }
}

pub(super) fn push_property_decl(
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

pub(super) fn push_field_decl(
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

pub(super) fn push_event_field_decl(
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

pub(super) fn push_delegate_decl(
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

// ---------------------------------------------------------------------------
// Import / using directive
// ---------------------------------------------------------------------------

pub(super) fn push_using_directive(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Skip `using Alias = ...` — these are type aliases, not namespace imports.
    if node.child_by_field_name("name").is_some() {
        return;
    }

    // Extract the full namespace path from the using directive and emit a
    // single Imports edge whose `module` IS the full namespace.
    // e.g. `using FamilyBudget.Api.Entities;` →
    //   target_name: "FamilyBudget.Api.Entities", module: Some("FamilyBudget.Api.Entities")
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = node_text(child, src);
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: name.clone(),
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: Some(name),
                    chain: None,
                });
                return;
            }
            "qualified_name" => {
                let full = node_text(child, src);
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: full.clone(),
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: Some(full),
                    chain: None,
                });
                return;
            }
            _ => {}
        }
    }
}
