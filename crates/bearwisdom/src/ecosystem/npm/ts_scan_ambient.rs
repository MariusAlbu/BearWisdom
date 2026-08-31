// =============================================================================
// ecosystem/npm/ts_scan_ambient.rs — `declare module '<name>'` header scan
//
// Collects each string-named ambient module block a declaration file exposes,
// paired with the names its body exports. Identifier-named blocks
// (`declare module Foo`, `namespace Foo`) are namespaces, not module
// specifiers, and are excluded. Shorthand declarations
// (`declare module 'my-shim';`) contribute the name with no inner names.
// =============================================================================

use std::collections::HashMap;

use tree_sitter::Node;

use super::ts_scan::{collect_file_exports, find_named_child, strip_quotes, FileExports};

/// Every `declare module '<name>'` (or top-level `module '<name>'`, the
/// `declare`-implicit `.d.ts` form) at the top level of `root`, as
/// `(declared name, sorted inner exported names)` in source order.
pub(super) fn scan_ambient_modules(
    root: &Node,
    bytes: &[u8],
    imports: &HashMap<String, (String, String)>,
) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "ambient_declaration" => {
                let mut ac = child.walk();
                for inner in child.children(&mut ac) {
                    if inner.kind() == "module" {
                        push_string_named_module(&inner, bytes, imports, &mut out);
                    }
                }
            }
            "module" => push_string_named_module(&child, bytes, imports, &mut out),
            _ => {}
        }
    }
    out
}

/// Record `module_node` when its name is a string literal: the declared name
/// plus the names its body surfaces (via the same per-statement classification
/// the file-level export pass uses; a body-less shorthand yields none).
fn push_string_named_module(
    module_node: &Node,
    bytes: &[u8],
    imports: &HashMap<String, (String, String)>,
    out: &mut Vec<(String, Vec<String>)>,
) {
    let Some(name_node) = module_node.child_by_field_name("name") else {
        return;
    };
    if name_node.kind() != "string" {
        return;
    }
    let Ok(raw) = name_node.utf8_text(bytes) else {
        return;
    };
    let name = strip_quotes(raw);
    if name.is_empty() || out.iter().any(|(n, _)| n == &name) {
        return;
    }

    let mut inner = FileExports::default();
    let body = module_node
        .child_by_field_name("body")
        .or_else(|| find_named_child(module_node, &["statement_block"]));
    if let Some(body) = body {
        let mut bc = body.walk();
        for stmt in body.children(&mut bc) {
            collect_file_exports(&stmt, bytes, &mut inner, imports);
        }
    }
    let mut names: Vec<String> = inner.named.into_keys().collect();
    names.sort();
    out.push((name, names));
}
