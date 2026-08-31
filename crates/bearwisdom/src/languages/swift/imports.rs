// =============================================================================
// swift/imports.rs — Imports-ref emission for Swift import declarations
// =============================================================================

use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

pub(super) fn push_import(
    node: &Node,
    src: &[u8],
    current_symbol_count: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut parts: Vec<String> = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_path_component" => {
                parts.push(node_text(child, src));
            }
            "identifier" => {
                // The `identifier` node itself spans a dotted path
                // (`simple_identifier ('.' simple_identifier)*`); walk its
                // own children so a two-segment path splits into its parts
                // instead of collapsing into one dotted string.
                let mut ic = child.walk();
                for seg in child.children(&mut ic) {
                    if seg.kind() == "simple_identifier" {
                        parts.push(node_text(seg, src));
                    }
                }
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        return;
    }
    let full = parts.join(".");
    // A single segment names the whole module (`import Foundation`), bringing
    // every one of its members into unqualified scope — the target is the
    // wildcard sentinel, module carries the module name. Two or more segments
    // name one declaration within a module (`import struct Foo.Bar`); the
    // target is the declaration's own bare name, module stays the full path
    // so `explicit_member_import` can match its last segment against it.
    let target = if parts.len() > 1 {
        parts.last().cloned().unwrap_or_else(|| full.clone())
    } else {
        "*".to_string()
    };
    refs.push(ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: current_symbol_count,
        target_name: target,
        kind: EdgeKind::Imports,
        line: node.start_position().row as u32,
        col: 0,
        module: Some(full),
        chain: None,
        byte_offset: node.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}
