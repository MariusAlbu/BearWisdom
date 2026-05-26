// =============================================================================
// rust/calls_imports.rs  —  `extern crate` and `use` declaration ref extraction
// =============================================================================

use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// extern crate import
// ---------------------------------------------------------------------------

/// Emit an `Imports` edge for `extern crate foo;`.
///
/// tree-sitter-rust shape:
/// ```text
/// extern_crate_declaration
///   "extern" "crate"
///   name: identifier  "foo"
///   ["as" alias: identifier]
/// ```
pub(super) fn extract_extern_crate(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, source);
    if name.is_empty() || name == "self" {
        return;
    }
    refs.push(ExtractedRef {
        source_symbol_index: current_symbol_count,
        target_name: name,
        kind: EdgeKind::Imports,
        line: name_node.start_position().row as u32,
        col: 0,
        module: None,
        chain: None,
        byte_offset: name_node.start_byte() as u32,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
});
}

// ---------------------------------------------------------------------------
// Use declaration / import reference extraction
// ---------------------------------------------------------------------------

/// Walk a `use_declaration` node and emit `Import` references for every
/// leaf name that is actually imported.
pub(super) fn extract_use_names(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "scoped_identifier"
            | "scoped_use_list"
            | "use_as_clause"
            | "use_wildcard"
            | "identifier"
            | "use_list" => {
                walk_use_tree(&child, source, refs, current_symbol_count, "");
            }
            _ => {}
        }
    }
}

fn walk_use_tree(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
    prefix: &str,
) {
    match node.kind() {
        "scoped_identifier" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(&n, source))
                .unwrap_or_default();
            let path = node
                .child_by_field_name("path")
                .map(|n| node_text(&n, source))
                .unwrap_or_default();

            if name.is_empty() {
                return;
            }

            let module = build_module_path(prefix, &path);
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: name,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                col: 0,
                module: if module.is_empty() { None } else { Some(module) },
                chain: None,
                byte_offset: node.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
});
        }

        "scoped_use_list" => {
            let path = node
                .child_by_field_name("path")
                .map(|n| node_text(&n, source))
                .unwrap_or_default();
            let new_prefix = build_module_path(prefix, &path);

            if let Some(list) = node.child_by_field_name("list") {
                walk_use_tree(&list, source, refs, current_symbol_count, &new_prefix);
            }
        }

        "use_list" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "{" | "}" | "," => {}
                    _ => walk_use_tree(&child, source, refs, current_symbol_count, prefix),
                }
            }
        }

        "use_as_clause" => {
            let alias = node
                .child_by_field_name("alias")
                .map(|n| node_text(&n, source));
            let original = node
                .child_by_field_name("path")
                .map(|n| node_text(&n, source));

            // `target_name` is the alias when present, otherwise the original name.
            let target = alias
                .clone()
                .or_else(|| original.clone())
                .unwrap_or_default();
            if target.is_empty() {
                return;
            }

            // For `use foo::bar as fb` at the top level (prefix=""), derive the
            // module from the original path: "foo::bar" → module="foo", name="bar".
            // When the alias IS the original (no `as` clause reached this arm), fall
            // back to prefix as before.
            let module = if alias.is_some() {
                // Aliased import: module = parent of the original full path.
                let orig = original.as_deref().unwrap_or("");
                let full = build_module_path(prefix, orig);
                let parent = full.rsplit_once("::").map(|(p, _)| p.to_string());
                parent
            } else if prefix.is_empty() {
                None
            } else {
                Some(prefix.to_string())
            };

            // Aliased imports carry the original name as a single-segment
            // chain so the SymbolIndex builder can register the alias as
            // a virtual entry pointing at the source symbol.
            let chain = if let (Some(a), Some(o)) = (alias.as_deref(), original.as_deref()) {
                if a != o && !o.is_empty() {
                    let leaf = o.rsplit("::").next().unwrap_or(o);
                    Some(crate::types::MemberChain {
                        segments: vec![crate::types::ChainSegment {
                            name: leaf.to_string(),
                            node_kind: "use_as_original".to_string(),
                            kind: crate::types::SegmentKind::Identifier,
                            declared_type: None,
                            type_args: Vec::new(),
                            optional_chaining: false,
                            byte_offset: node.start_byte() as u32,
                            declared_type_id: None,
                            is_call: false,
                            type_arg_ids: Vec::new(),
                        }],
                    })
                } else {
                    None
                }
            } else {
                None
            };

            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: target,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                col: 0,
                module,
                chain,
                byte_offset: node.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
});
        }

        "use_wildcard" => {
            let module = if prefix.is_empty() {
                None
            } else {
                Some(prefix.to_string())
            };
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: "*".to_string(),
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                col: 0,
                module,
                chain: None,
                byte_offset: node.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
});
        }

        "identifier" => {
            let name = node_text(node, source);
            if name.is_empty() {
                return;
            }
            let module = if prefix.is_empty() {
                None
            } else {
                Some(prefix.to_string())
            };
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: name,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                col: 0,
                module,
                chain: None,
                byte_offset: node.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
});
        }

        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk_use_tree(&child, source, refs, current_symbol_count, prefix);
            }
        }
    }
}

fn build_module_path(prefix: &str, path: &str) -> String {
    match (prefix.is_empty(), path.is_empty()) {
        (true, true) => String::new(),
        (true, false) => path.to_string(),
        (false, true) => prefix.to_string(),
        (false, false) => format!("{prefix}::{path}"),
    }
}
