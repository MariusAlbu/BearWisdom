// =============================================================================
// javascript/helpers.rs  —  Shared utilities for the JavaScript extractor
// =============================================================================

use crate::types::Visibility;
use tree_sitter::Node;

pub(super) fn node_text(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}

pub(super) fn detect_visibility(node: &Node, src: &[u8]) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "export" {
            return Some(Visibility::Public);
        }
        let text = node_text(child, src);
        match text.as_str() {
            "public" => return Some(Visibility::Public),
            "private" => return Some(Visibility::Private),
            _ => {}
        }
    }

    // `export function foo()` / `export class Foo {}` / `export const x = ...`
    // wrap the declaration in an `export_statement` (or
    // `export_default_declaration`). The `export` keyword is a sibling of
    // the declaration node, not a child — without walking the parent chain
    // we'd tag every ES-module export `visibility = NULL` and the
    // dead-code `exported_api` entry-point contributor would miss it.
    if let Some(parent) = node.parent() {
        if parent.kind() == "export_statement" || parent.kind() == "export_default_declaration" {
            return Some(Visibility::Public);
        }
        // `export const x = () => {}` lowers two layers deep
        // (export_statement → lexical_declaration → variable_declarator →
        // arrow_function), so the arrow's parent is the declarator and
        // the export_statement is the grandparent.
        if let Some(grandparent) = parent.parent() {
            if grandparent.kind() == "export_statement"
                || grandparent.kind() == "export_default_declaration"
            {
                return Some(Visibility::Public);
            }
        }
    }
    None
}

pub(super) fn extract_jsdoc(node: &Node, src: &[u8]) -> Option<String> {
    let sib = node.prev_sibling()?;
    if sib.kind() == "comment" {
        let text = node_text(sib, src);
        if text.starts_with("/**") {
            return Some(text);
        }
    }
    None
}
