// =============================================================================
// php/symbols.rs  —  Symbol extractors for PHP
// =============================================================================

use super::calls::{extract_calls_from_body, extract_trait_use};
use super::helpers::{
    build_class_signature, build_method_signature, extract_visibility, node_text, qualify,
    qualify_ns, scope_from_prefix,
};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::Node;

pub(super) fn extract_namespace(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify_ns(&name, qualified_prefix);
    let new_prefix = qualified_name.clone();
    let ns_prefix = name.replace('\\', ".");

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Namespace,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("namespace {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    if let Some(body) = node.child_by_field_name("body") {
        super::extract_from_node(body, src, symbols, refs, Some(idx), &new_prefix, &ns_prefix);
    }
}

pub(super) fn extract_class(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    namespace_prefix: &str,
    kind: SymbolKind,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify(&name, qualified_prefix);
    let new_prefix = qualified_name.clone();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: build_class_signature(node, src, &name, kind),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // Scan children for inheritance/implements (tree-sitter-php 0.24 unnamed children)
    let mut cc = node.walk();
    for child in node.children(&mut cc) {
        match child.kind() {
            "base_clause" => {
                let mut bc = child.walk();
                for base_child in child.children(&mut bc) {
                    if base_child.kind() == "qualified_name"
                        || base_child.kind() == "name"
                        || base_child.kind() == "identifier"
                    {
                        refs.push(ExtractedRef {
                            source_symbol_index: idx,
                            target_name: node_text(&base_child, src),
                            kind: EdgeKind::Inherits,
                            line: base_child.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
            }
            "class_interface_clause" => {
                extract_interface_list(&child, src, refs, idx, EdgeKind::Implements);
            }
            _ => {}
        }
    }

    // Legacy field-based fallback for older grammar versions
    if refs.iter().all(|r| r.source_symbol_index != idx || r.kind != EdgeKind::Inherits) {
        if let Some(base) = node.child_by_field_name("base_clause") {
            let mut c = base.walk();
            for bc in base.children(&mut c) {
                if bc.kind() == "qualified_name" || bc.kind() == "name" || bc.kind() == "identifier" {
                    refs.push(ExtractedRef {
                        source_symbol_index: idx,
                        target_name: node_text(&bc, src),
                        kind: EdgeKind::Inherits,
                        line: bc.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
        }
    }
    if let Some(impls) = node.child_by_field_name("class_implements") {
        extract_interface_list(&impls, src, refs, idx, EdgeKind::Implements);
    }

    // Recurse into body
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "declaration_list" {
            extract_class_body(&child, src, symbols, refs, Some(idx), &new_prefix, namespace_prefix);
        }
    }
}

pub(super) fn extract_interface_list(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    class_idx: usize,
    edge_kind: EdgeKind,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "qualified_name" || child.kind() == "name" || child.kind() == "identifier" {
            refs.push(ExtractedRef {
                source_symbol_index: class_idx,
                target_name: node_text(&child, src),
                kind: edge_kind,
                line: child.start_position().row as u32,
                module: None,
                chain: None,
            });
        } else {
            extract_interface_list(&child, src, refs, class_idx, edge_kind);
        }
    }
}

pub(super) fn extract_class_body(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    namespace_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "method_declaration" => {
                extract_method(&child, src, symbols, refs, parent_index, qualified_prefix);
            }
            "property_declaration" => {
                extract_property_declaration(&child, src, symbols, parent_index, qualified_prefix);
            }
            "use_declaration" => {
                extract_trait_use(&child, src, refs, symbols.len());
            }
            "const_declaration" => {
                extract_const_declaration(&child, src, symbols, parent_index, qualified_prefix);
            }
            "enum_declaration" => {
                extract_enum(&child, src, symbols, refs, parent_index, qualified_prefix, namespace_prefix);
            }
            _ => {}
        }
    }
}

pub(super) fn extract_method(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = extract_visibility(node, src);

    let kind = if name == "__construct" {
        SymbolKind::Constructor
    } else {
        SymbolKind::Method
    };

    let signature = build_method_signature(node, src, &name);

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
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, src, idx, refs);
    }
}

pub(super) fn extract_function(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    _namespace_prefix: &str,
    _inside_class: bool,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify(&name, qualified_prefix);
    let signature = build_method_signature(node, src, &name);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, src, idx, refs);
    }
}

pub(super) fn extract_property_declaration(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let visibility = extract_visibility(node, src);

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "property_element" {
            let mut vc = child.walk();
            for var in child.children(&mut vc) {
                if var.kind() == "variable_name" || var.kind() == "$variable_name" {
                    let raw = node_text(&var, src);
                    let name = raw.trim_start_matches('$').to_string();
                    let qualified_name = qualify(&name, qualified_prefix);
                    symbols.push(ExtractedSymbol {
                        name,
                        qualified_name,
                        kind: SymbolKind::Property,
                        visibility,
                        start_line: var.start_position().row as u32,
                        end_line: node.end_position().row as u32,
                        start_col: var.start_position().column as u32,
                        end_col: node.end_position().column as u32,
                        signature: None,
                        doc_comment: None,
                        scope_path: scope_from_prefix(qualified_prefix),
                        parent_index,
                    });
                    break;
                }
            }
        }
    }
}

pub(super) fn extract_const_declaration(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "const_element" {
            if let Some(name_node) = child.child_by_field_name("name") {
                let name = node_text(&name_node, src);
                let qualified_name = qualify(&name, qualified_prefix);
                symbols.push(ExtractedSymbol {
                    name,
                    qualified_name,
                    kind: SymbolKind::Field,
                    visibility: Some(Visibility::Public),
                    start_line: child.start_position().row as u32,
                    end_line: child.end_position().row as u32,
                    start_col: child.start_position().column as u32,
                    end_col: child.end_position().column as u32,
                    signature: None,
                    doc_comment: None,
                    scope_path: scope_from_prefix(qualified_prefix),
                    parent_index,
                });
            }
        }
    }
}

pub(super) fn extract_enum(
    node: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    _namespace_prefix: &str,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, src);
    let qualified_name = qualify(&name, qualified_prefix);
    let new_prefix = qualified_name.clone();

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Enum,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("enum {name}")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    if let Some(impls) = node.child_by_field_name("class_implements") {
        extract_interface_list(&impls, src, refs, idx, EdgeKind::Implements);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "enum_declaration_list" {
            let mut lc = child.walk();
            for item in child.children(&mut lc) {
                match item.kind() {
                    "enum_case" => {
                        if let Some(nm) = item.child_by_field_name("name") {
                            let case_name = node_text(&nm, src);
                            let case_qn = qualify(&case_name, &new_prefix);
                            symbols.push(ExtractedSymbol {
                                name: case_name,
                                qualified_name: case_qn,
                                kind: SymbolKind::EnumMember,
                                visibility: Some(Visibility::Public),
                                start_line: item.start_position().row as u32,
                                end_line: item.end_position().row as u32,
                                start_col: item.start_position().column as u32,
                                end_col: item.end_position().column as u32,
                                signature: None,
                                doc_comment: None,
                                scope_path: Some(new_prefix.clone()),
                                parent_index: Some(idx),
                            });
                        }
                    }
                    "method_declaration" => {
                        extract_method(&item, src, symbols, refs, Some(idx), &new_prefix);
                    }
                    _ => {}
                }
            }
        }
    }
}
