// =============================================================================
// kotlin/helpers.rs  —  Shared utilities for the Kotlin extractor
// =============================================================================

use crate::parser::scope_tree;
use crate::types::{SymbolKind, Visibility};
use tree_sitter::Node;

pub(super) fn node_text(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}

/// Determine the `SymbolKind` for a `class_declaration` by inspecting modifiers.
pub(super) fn classify_class(node: &Node, src: &[u8]) -> SymbolKind {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let text = node_text(child, src);
            if text.contains("enum") {
                return SymbolKind::Enum;
            }
        }
    }
    let full_text = node_text(*node, src);
    if full_text.trim_start().starts_with("enum") {
        return SymbolKind::Enum;
    }
    SymbolKind::Class
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
        "simple_identifier" | "identifier" => node_text(*node, src),
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

/// Infer visibility from modifier keywords.
pub(super) fn detect_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let text = node_text(child, src);
            if text.contains("public")    { return Some(Visibility::Public);    }
            if text.contains("private")   { return Some(Visibility::Private);   }
            if text.contains("protected") { return Some(Visibility::Protected); }
            if text.contains("internal")  { return Some(Visibility::Internal);  }
            return None;
        }
    }
    None
}

pub(super) fn extract_doc_comment(node: &Node, src: &[u8]) -> Option<String> {
    let mut sib = node.prev_sibling();
    while let Some(s) = sib {
        let text = node_text(s, src);
        let trimmed = text.trim_start();
        if trimmed.starts_with("/**") {
            return Some(text);
        }
        if trimmed.starts_with("/*") || trimmed.is_empty() {
            sib = s.prev_sibling();
            continue;
        }
        break;
    }
    None
}
