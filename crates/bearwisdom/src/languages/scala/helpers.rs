// =============================================================================
// scala/helpers.rs  —  Shared utilities for the Scala extractor
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

/// Decide whether a class_definition is a case class, sealed class, etc.
pub(super) fn classify_class(_node: &Node, _src: &[u8]) -> SymbolKind {
    // case class → still Class; Scala 3 enums are parsed via enum_definition.
    SymbolKind::Class
}

pub(super) fn call_target_name(node: &Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" | "type_identifier" => node_text(*node, src),
        "field_expression" | "select_expression" | "field_access" => {
            node.child_by_field_name("field")
                .or_else(|| node.child_by_field_name("name"))
                .map(|n| node_text(n, src))
                .unwrap_or_default()
        }
        // `identity[String](...)` — generic_function wraps an identifier and type args.
        // The `function` field holds the base function name.
        "generic_function" => {
            node.child_by_field_name("function")
                .map(|n| call_target_name(&n, src))
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

pub(super) fn detect_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let text = node_text(child, src);
            if text.contains("private")   { return Some(Visibility::Private);   }
            if text.contains("protected") { return Some(Visibility::Protected); }
            if text.contains("public")    { return Some(Visibility::Public);    }
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

pub(super) fn type_name_from_node(node: &Node, src: &[u8]) -> String {
    match node.kind() {
        "type_identifier" | "identifier" => node_text(*node, src),
        "generic_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_identifier" || child.kind() == "identifier" {
                    return node_text(child, src);
                }
            }
            String::new()
        }
        "compound_type" | "annotated_type" | "with_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let n = type_name_from_node(&child, src);
                if !n.is_empty() { return n; }
            }
            String::new()
        }
        _ => String::new(),
    }
}
