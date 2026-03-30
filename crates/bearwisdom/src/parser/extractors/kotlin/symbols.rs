// =============================================================================
// kotlin/symbols.rs  —  Symbol pushers and import/delegation helpers for Kotlin
// =============================================================================

use super::helpers::{
    detect_visibility, enclosing_scope, extract_doc_comment, find_child_by_kind, node_text,
};
use crate::parser::scope_tree;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Class body dispatchers
// ---------------------------------------------------------------------------

pub(super) fn extract_class_body(
    class_node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = class_node.walk();
    for child in class_node.children(&mut cursor) {
        match child.kind() {
            "class_body" => {
                super::extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }
            "enum_class_body" => {
                extract_enum_class_body(&child, src, scope_tree, symbols, refs, parent_index);
            }
            _ => {}
        }
    }
}

pub(super) fn extract_enum_class_body(
    body: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let enum_qname = parent_index
        .and_then(|i| symbols.get(i))
        .map(|s| s.qualified_name.clone())
        .unwrap_or_default();

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "enum_entry" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(name_node, src);
                    let qualified_name = if enum_qname.is_empty() {
                        name.clone()
                    } else {
                        format!("{enum_qname}.{name}")
                    };
                    symbols.push(ExtractedSymbol {
                        name,
                        qualified_name,
                        kind: SymbolKind::EnumMember,
                        visibility: None,
                        start_line: child.start_position().row as u32,
                        end_line: child.end_position().row as u32,
                        start_col: child.start_position().column as u32,
                        end_col: child.end_position().column as u32,
                        signature: None,
                        doc_comment: None,
                        scope_path: if enum_qname.is_empty() { None } else { Some(enum_qname.clone()) },
                        parent_index,
                    });
                }
            }
            _ => {
                super::extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol pushers
// ---------------------------------------------------------------------------

pub(super) fn push_type_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    kind: SymbolKind,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let kw = match kind {
        SymbolKind::Class     => "class",
        SymbolKind::Interface => "interface",
        SymbolKind::Enum      => "enum class",
        _                     => "class",
    };

    let visibility = detect_visibility(node, src);
    let type_params = node
        .child_by_field_name("type_parameters")
        .map(|tp| node_text(tp, src))
        .unwrap_or_default();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{kw} {name}{type_params}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_function_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(name_node, src);

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let kind = if scope.is_some() {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    let params = node
        .child_by_field_name("function_value_parameters")
        .or_else(|| find_child_by_kind(node, "value_arguments"))
        .map(|p| node_text(p, src))
        .unwrap_or_default();
    let ret = node
        .child_by_field_name("type")
        .map(|t| format!(": {}", node_text(t, src)))
        .unwrap_or_default();
    let signature = Some(format!("fun {name}{params}{ret}"));

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
        signature,
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_property_decl(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None    => return,
    };
    let name = node_text(name_node, src);

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let kw = if node_text(*node, src).trim_start().starts_with("val") { "val" } else { "var" };
    let ty = node
        .child_by_field_name("type")
        .map(|t| format!(": {}", node_text(t, src)))
        .unwrap_or_default();

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Property,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{kw} {name}{ty}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
}

pub(super) fn push_secondary_constructor(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let class_name = scope.map(|s| s.name.as_str()).unwrap_or("constructor").to_string();
    let qualified_name = scope_tree::qualify(&class_name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let params = find_child_by_kind(node, "function_value_parameters")
        .map(|p| node_text(p, src))
        .unwrap_or_default();

    symbols.push(ExtractedSymbol {
        name: class_name.clone(),
        qualified_name,
        kind: SymbolKind::Constructor,
        visibility: detect_visibility(node, src),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("constructor{params}")),
        doc_comment: None,
        scope_path,
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

pub(super) fn extract_imports(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "import_header" {
            emit_import(&child, src, current_symbol_count, refs);
        }
    }
}

pub(super) fn emit_import(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "qualified_identifier" => {
                let mut parts: Vec<String> = Vec::new();
                let mut ic = child.walk();
                for id in child.children(&mut ic) {
                    if id.kind() == "identifier" {
                        parts.push(node_text(id, src));
                    }
                }
                if parts.is_empty() {
                    let full = node_text(child, src);
                    let target = full.rsplit('.').next().unwrap_or(&full).to_string();
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: target,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module: Some(full),
                        chain: None,
                    });
                } else {
                    let target = parts.last().cloned().unwrap_or_default();
                    let full = parts.join(".");
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: target,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module: Some(full),
                        chain: None,
                    });
                }
                return;
            }
            "identifier" => {
                let full = node_text(child, src);
                let target = full.rsplit('.').next().unwrap_or(&full).to_string();
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: target,
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

// ---------------------------------------------------------------------------
// Inheritance / interface delegation
// ---------------------------------------------------------------------------

pub(super) fn extract_delegation_specifiers(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    let mut first_super = true;
    for child in node.children(&mut cursor) {
        if child.kind() == "delegation_specifiers" {
            let mut dc = child.walk();
            for spec in child.children(&mut dc) {
                match spec.kind() {
                    "delegation_specifier" | "annotated_delegation_specifier" => {
                        if let Some(name) = delegation_spec_name(&spec, src) {
                            let kind = if first_super {
                                first_super = false;
                                EdgeKind::Inherits
                            } else {
                                EdgeKind::Implements
                            };
                            refs.push(ExtractedRef {
                                source_symbol_index: source_idx,
                                target_name: name,
                                kind,
                                line: spec.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn delegation_spec_name(node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "constructor_invocation" => {
                let mut cc = child.walk();
                for inner in child.children(&mut cc) {
                    if inner.kind() == "user_type" {
                        return last_simple_identifier_in_user_type(&inner, src);
                    }
                }
            }
            "user_type" => {
                return last_simple_identifier_in_user_type(&child, src);
            }
            "simple_identifier" | "type_identifier" => {
                return Some(node_text(child, src));
            }
            _ => {}
        }
    }
    None
}

fn last_simple_identifier_in_user_type(node: &Node, src: &[u8]) -> Option<String> {
    let mut last: Option<String> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "simple_user_type" => {
                if let Some(id_node) = child.child_by_field_name("name") {
                    last = Some(node_text(id_node, src));
                } else {
                    let mut ic = child.walk();
                    for inner in child.children(&mut ic) {
                        if inner.kind() == "simple_identifier" || inner.kind() == "identifier" {
                            last = Some(node_text(inner, src));
                            break;
                        }
                    }
                }
            }
            "identifier" | "simple_identifier" => {
                last = Some(node_text(child, src));
            }
            _ => {}
        }
    }
    last
}
