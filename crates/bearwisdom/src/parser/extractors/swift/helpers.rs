// =============================================================================
// swift/helpers.rs  —  Shared utilities for the Swift extractor
// =============================================================================

use crate::parser::scope_tree;
use crate::types::{SymbolKind, Visibility};
use tree_sitter::Node;

pub(super) fn node_text(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}

pub(super) fn enclosing_scope<'a>(
    tree: &'a scope_tree::ScopeTree,
    start: usize,
    end: usize,
) -> Option<&'a scope_tree::ScopeEntry> {
    scope_tree::find_enclosing_scope(tree, start, end)
}

pub(super) fn call_target_name(node: &Node, src: &[u8]) -> String {
    match node.kind() {
        "simple_identifier" | "identifier" | "type_identifier" => node_text(*node, src),
        "navigation_expression" => {
            let mut cursor = node.walk();
            let mut last = String::new();
            for child in node.children(&mut cursor) {
                if child.kind() == "navigation_suffix" {
                    let mut nc = child.walk();
                    for inner in child.children(&mut nc) {
                        if inner.kind() == "simple_identifier" {
                            last = node_text(inner, src);
                        }
                    }
                }
            }
            last
        }
        _ => String::new(),
    }
}

pub(super) fn find_child_by_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == kind {
                return Some(child);
            }
        }
    }
    None
}

pub(super) fn detect_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "modifier" => {
                let text = node_text(child, src);
                match text.trim() {
                    "public"      => return Some(Visibility::Public),
                    "private"     => return Some(Visibility::Private),
                    "fileprivate" => return Some(Visibility::Private),
                    "internal"    => return Some(Visibility::Internal),
                    _             => {}
                }
            }
            "visibility_modifier" | "access_level_modifier" => {
                let text = node_text(child, src);
                if text.contains("public")      { return Some(Visibility::Public);   }
                if text.contains("private")     { return Some(Visibility::Private);  }
                if text.contains("fileprivate") { return Some(Visibility::Private);  }
                if text.contains("internal")    { return Some(Visibility::Internal); }
            }
            _ => {}
        }
    }
    None
}

pub(super) fn extract_doc_comment(node: &Node, src: &[u8]) -> Option<String> {
    let mut sib = node.prev_sibling();
    while let Some(s) = sib {
        let text = node_text(s, src);
        let trimmed = text.trim_start();
        if trimmed.starts_with("/**") || trimmed.starts_with("///") {
            return Some(text);
        }
        if trimmed.starts_with("/*") || trimmed.starts_with("//") || trimmed.is_empty() {
            sib = s.prev_sibling();
            continue;
        }
        break;
    }
    None
}

/// Determine the Swift symbol kind from a `class_declaration` node.
pub(super) fn swift_type_decl_kind(node: &Node, src: &[u8]) -> SymbolKind {
    if let Some(kw_node) = node.child_by_field_name("declaration_kind") {
        return match kw_node.kind() {
            "struct"    => SymbolKind::Struct,
            "enum"      => SymbolKind::Enum,
            "extension" => SymbolKind::Namespace,
            "actor"     => SymbolKind::Class,
            _           => SymbolKind::Class,
        };
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "struct"    => return SymbolKind::Struct,
            "enum"      => return SymbolKind::Enum,
            "class"     => return SymbolKind::Class,
            "actor"     => return SymbolKind::Class,
            "extension" => return SymbolKind::Namespace,
            _           => {}
        }
    }
    SymbolKind::Class
}

pub(super) fn inherited_type_name(node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "user_type" => {
                let mut last: Option<String> = None;
                let mut uc = child.walk();
                for ut in child.children(&mut uc) {
                    if ut.kind() == "simple_user_type" {
                        if let Some(id) = ut
                            .child_by_field_name("name")
                            .or_else(|| find_child_by_kind(&ut, "simple_identifier"))
                            .or_else(|| find_child_by_kind(&ut, "type_identifier"))
                        {
                            last = Some(node_text(id, src));
                        }
                    }
                }
                if last.is_some() {
                    return last;
                }
                return Some(node_text(child, src));
            }
            "type_identifier" | "simple_identifier" => {
                return Some(node_text(child, src));
            }
            _ => {}
        }
    }
    None
}
