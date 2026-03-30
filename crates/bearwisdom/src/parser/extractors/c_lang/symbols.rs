// =============================================================================
// c_lang/symbols.rs  —  Symbol pushers for C/C++
// =============================================================================

use super::helpers::{
    detect_visibility, enclosing_scope, extract_doc_comment, extract_declarator_name,
    find_child_by_kind, first_type_identifier, is_constructor_name, node_text,
};
use crate::parser::scope_tree;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

pub(super) fn push_function_def(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    language: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let decl_node = node.child_by_field_name("declarator")?;
    let (name, is_destructor) = extract_declarator_name(&decl_node, src);
    let name = name?;

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);

    let kind = if is_destructor {
        SymbolKind::Method
    } else if language != "c" && is_constructor_name(&name, scope) {
        SymbolKind::Constructor
    } else if scope.is_some() {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    let visibility = detect_visibility(node, src);
    let ret_type = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();
    let params = decl_node
        .child_by_field_name("parameters")
        .or_else(|| find_child_by_kind(&decl_node, "parameter_list"))
        .map(|p| node_text(p, src))
        .unwrap_or_default();
    let signature = Some(format!("{ret_type} {name}{params}").trim().to_string());

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility,
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

pub(super) fn push_specifier(
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
        SymbolKind::Class  => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum   => "enum",
        _                  => "struct",
    };

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
        signature: Some(format!("{kw} {name}")),
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_namespace(
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
        doc_comment: extract_doc_comment(node, src),
        scope_path,
        parent_index,
    });
    Some(idx)
}

pub(super) fn push_typedef(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "pointer_declarator" | "function_declarator" => {
                let name = first_type_identifier(&child, src);
                if let Some(name) = name {
                    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
                    let qualified_name = scope_tree::qualify(&name, scope);
                    let scope_path = scope_tree::scope_path(scope);

                    symbols.push(ExtractedSymbol {
                        name: name.clone(),
                        qualified_name,
                        kind: SymbolKind::TypeAlias,
                        visibility: None,
                        start_line: node.start_position().row as u32,
                        end_line: node.end_position().row as u32,
                        start_col: node.start_position().column as u32,
                        end_col: node.end_position().column as u32,
                        signature: Some(format!("typedef {name}")),
                        doc_comment: extract_doc_comment(node, src),
                        scope_path,
                        parent_index,
                    });
                }
                return;
            }
            _ => {}
        }
    }
}

pub(super) fn push_declaration(
    node: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let type_str = node
        .child_by_field_name("type")
        .map(|t| node_text(t, src))
        .unwrap_or_default();

    let scope = enclosing_scope(scope_tree, node.start_byte(), node.end_byte());
    let scope_path = scope_tree::scope_path(scope);

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let name_opt = match child.kind() {
            "identifier" => Some(node_text(child, src)),
            "init_declarator" | "pointer_declarator" => first_type_identifier(&child, src),
            _ => None,
        };
        if let Some(name) = name_opt {
            let qualified_name = scope_tree::qualify(&name, scope);
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name,
                kind: SymbolKind::Variable,
                visibility: detect_visibility(node, src),
                start_line: child.start_position().row as u32,
                end_line: child.end_position().row as u32,
                start_col: child.start_position().column as u32,
                end_col: child.end_position().column as u32,
                signature: Some(format!("{type_str} {name}")),
                doc_comment: None,
                scope_path: scope_path.clone(),
                parent_index,
            });
        }
    }
}

pub(super) fn extract_enum_body(
    body: &Node,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let enum_qname = parent_index
        .and_then(|i| symbols.get(i))
        .map(|s| s.qualified_name.clone())
        .unwrap_or_default();

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "enumerator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = node_text(name_node, src);
                let qualified_name = if enum_qname.is_empty() {
                    name.clone()
                } else {
                    format!("{enum_qname}.{name}")
                };
                let scope = enclosing_scope(scope_tree, child.start_byte(), child.end_byte());
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
                    scope_path: scope_tree::scope_path(scope),
                    parent_index,
                });
            }
        }
    }
}

pub(super) fn push_include(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "string_literal" | "system_lib_string" => {
                let raw = node_text(child, src);
                let path = raw.trim_matches('"').trim_matches('<').trim_matches('>');
                let target_name = path.rsplit('/').next().unwrap_or(path).to_string();
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name,
                    kind: EdgeKind::Imports,
                    line: node.start_position().row as u32,
                    module: Some(path.to_string()),
                    chain: None,
                });
                return;
            }
            _ => {}
        }
    }
}

pub(super) fn extract_bases(
    node: &Node,
    src: &[u8],
    source_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "base_class_clause" {
            let mut bc = child.walk();
            for base in child.children(&mut bc) {
                match base.kind() {
                    "type_identifier" => {
                        let name = node_text(base, src);
                        refs.push(ExtractedRef {
                            source_symbol_index: source_idx,
                            target_name: name,
                            kind: EdgeKind::Inherits,
                            line: base.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                    "base_class_specifier" => {
                        let mut ic = base.walk();
                        for inner in base.children(&mut ic) {
                            if inner.kind() == "type_identifier" {
                                let name = node_text(inner, src);
                                refs.push(ExtractedRef {
                                    source_symbol_index: source_idx,
                                    target_name: name,
                                    kind: EdgeKind::Inherits,
                                    line: inner.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
