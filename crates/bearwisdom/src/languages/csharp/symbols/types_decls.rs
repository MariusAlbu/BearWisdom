// =============================================================================
// csharp/symbols/types_decls.rs  —  container declarations (namespace, type, enum)
//
// Pushes Namespace, Class/Struct/Interface/Record (via push_type_decl), and
// Enum/EnumMember symbols. Record primary constructor params are emitted by
// extract_record_primary_params, called from extract.rs after push_type_decl.
// =============================================================================

use super::super::helpers::{
    collect_type_param_constraints, detect_visibility, extract_doc_comment, find_child_kind,
    node_text,
};
use crate::parser::scope_tree::{self, ScopeTree};
use crate::types::{ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

pub(in super::super) fn push_namespace(
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
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });
    Some(idx)
}

pub(in super::super) fn push_type_decl(
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
    let constraints = collect_type_param_constraints(node, src);

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
        signature: Some(format!("{keyword} {name}{type_params}{constraints}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });
    Some(idx)
}

/// Extract primary constructor parameters of a record as Property symbols.
///
/// `record Point(int X, int Y)` — `X` and `Y` are synthesised as public
/// init-only properties by the compiler.  We extract them so the index
/// knows they exist (they won't appear in a body as `property_declaration`).
pub(in super::super) fn extract_record_primary_params(
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
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        });
    }
}

pub(in super::super) fn push_enum_decl(
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
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
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
                        byte_offset: 0,
                        declared_type: None,
                        return_type: None,
                        param_types: Vec::new(),
                        generic_params: Vec::new(),
                    });
                }
            }
        }
    }

    Some(idx)
}
