// =============================================================================
// c_lang/helpers.rs  —  Shared utilities for the C/C++ extractor
// =============================================================================

use crate::parser::scope_tree;
use crate::types::Visibility;
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

/// Check if a function name matches the enclosing class name (constructor heuristic).
pub(super) fn is_constructor_name(name: &str, scope: Option<&scope_tree::ScopeEntry>) -> bool {
    scope.map(|s| s.name.as_str() == name).unwrap_or(false)
}

/// Best-effort visibility from access specifiers.
pub(super) fn detect_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    let mut sib = node.prev_sibling();
    while let Some(s) = sib {
        match s.kind() {
            "access_specifier" => {
                let text = node_text(s, src);
                let text = text.trim_end_matches(':').trim();
                return match text {
                    "public"    => Some(Visibility::Public),
                    "private"   => Some(Visibility::Private),
                    "protected" => Some(Visibility::Protected),
                    _           => None,
                };
            }
            "{" => break,
            _ => {}
        }
        sib = s.prev_sibling();
    }
    None
}

pub(super) fn extract_doc_comment(node: &Node, src: &[u8]) -> Option<String> {
    let mut sib = node.prev_sibling();
    while let Some(s) = sib {
        match s.kind() {
            "comment" => {
                let text = node_text(s, src);
                let trimmed = text.trim_start();
                if trimmed.starts_with("/**") || trimmed.starts_with("///") {
                    return Some(text);
                }
                if trimmed.starts_with("/*") || trimmed.starts_with("//") {
                    sib = s.prev_sibling();
                    continue;
                }
                break;
            }
            _ => break,
        }
    }
    None
}

/// Extract the simple function/method name from a declarator chain.
///
/// Returns `(name, is_destructor)`.
pub(super) fn extract_declarator_name(node: &Node, src: &[u8]) -> (Option<String>, bool) {
    match node.kind() {
        "identifier" | "type_identifier" | "field_identifier" => {
            (Some(node_text(*node, src)), false)
        }
        "destructor_name" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let n = node_text(child, src);
                    return (Some(format!("~{n}")), true);
                }
            }
            (None, true)
        }
        "qualified_identifier" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                return extract_declarator_name(&name_node, src);
            }
            (None, false)
        }
        "function_declarator" => {
            if let Some(d) = node.child_by_field_name("declarator") {
                return extract_declarator_name(&d, src);
            }
            (None, false)
        }
        "pointer_declarator" | "reference_declarator" => {
            if let Some(d) = node.child_by_field_name("declarator") {
                return extract_declarator_name(&d, src);
            }
            (None, false)
        }
        _ => (None, false),
    }
}

pub(super) fn call_target_name(node: &Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" | "field_identifier" => node_text(*node, src),
        "field_expression" => {
            node.child_by_field_name("field")
                .map(|f| node_text(f, src))
                .unwrap_or_default()
        }
        "qualified_identifier" => {
            node.child_by_field_name("name")
                .map(|n| node_text(n, src))
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

/// Walk a node tree looking for the first `type_identifier` leaf.
pub(super) fn first_type_identifier(node: &Node, src: &[u8]) -> Option<String> {
    if node.kind() == "type_identifier" || node.kind() == "identifier" {
        return Some(node_text(*node, src));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(n) = first_type_identifier(&child, src) {
            return Some(n);
        }
    }
    None
}
