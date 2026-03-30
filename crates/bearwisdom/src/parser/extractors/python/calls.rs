// =============================================================================
// python/calls.rs  —  Call extraction and import helpers for Python
// =============================================================================

use super::helpers::node_text;
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

pub(super) fn extract_calls_from_body(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call" {
            if let Some(func_node) = child.child_by_field_name("function") {
                let chain = build_chain(&func_node, source);
                let target_name = chain
                    .as_ref()
                    .and_then(|c| c.segments.last())
                    .map(|s| s.name.clone())
                    .or_else(|| {
                        let t = node_text(&func_node, source);
                        Some(t.rsplit('.').next().unwrap_or(&t).to_string())
                    });

                if let Some(target_name) = target_name {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name,
                        kind: EdgeKind::Calls,
                        line: func_node.start_position().row as u32,
                        module: None,
                        chain,
                    });
                }
            }
        }
        extract_calls_from_body(&child, source, source_symbol_index, refs);
    }
}

// ---------------------------------------------------------------------------
// Member chain builder
// ---------------------------------------------------------------------------

pub(super) fn build_chain(node: &Node, src: &str) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: &Node, src: &str, segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, src);
            let kind = if name == "self" || name == "cls" {
                SegmentKind::SelfRef
            } else {
                SegmentKind::Identifier
            };
            segments.push(ChainSegment {
                name,
                node_kind: "identifier".to_string(),
                kind,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "attribute" => {
            let object = node.child_by_field_name("object")?;
            let attribute = node.child_by_field_name("attribute")?;
            build_chain_inner(&object, src, segments)?;
            segments.push(ChainSegment {
                name: node_text(&attribute, src),
                node_kind: "attribute".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "call" => {
            let func = node.child_by_field_name("function")?;
            build_chain_inner(&func, src, segments)
        }

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Import extraction
// ---------------------------------------------------------------------------

pub(super) fn extract_import_statement(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "dotted_name" => {
                let full = node_text(&child, source);
                let parts: Vec<&str> = full.split('.').collect();
                let target = parts.last().unwrap_or(&full.as_str()).to_string();
                let module = if parts.len() > 1 {
                    Some(parts[..parts.len() - 1].join("."))
                } else {
                    None
                };
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: target,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module,
                    chain: None,
                });
            }
            "aliased_import" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let full = node_text(&name_node, source);
                    let parts: Vec<&str> = full.split('.').collect();
                    let target = parts.last().unwrap_or(&full.as_str()).to_string();
                    let module = if parts.len() > 1 {
                        Some(parts[..parts.len() - 1].join("."))
                    } else {
                        None
                    };
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: target,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module,
                        chain: None,
                    });
                }
            }
            _ => {}
        }
    }
}

pub(super) fn extract_import_from_statement(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let module = node.child_by_field_name("module_name").map(|m| {
        node_text(&m, source).trim_start_matches('.').to_string()
    });

    let module_name_node = node.child_by_field_name("module_name");

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "from" | "import" | "," | "import_prefix" => continue,
            _ => {}
        }
        if let Some(ref mn) = module_name_node {
            if child.id() == mn.id() {
                continue;
            }
        }

        match child.kind() {
            "dotted_name" | "identifier" => {
                let name = node_text(&child, source);
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: name,
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: module.clone(),
                    chain: None,
                });
            }
            "aliased_import" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(&name_node, source);
                    refs.push(ExtractedRef {
                        source_symbol_index: current_symbol_count,
                        target_name: name,
                        kind: EdgeKind::Imports,
                        line: child.start_position().row as u32,
                        module: module.clone(),
                        chain: None,
                    });
                }
            }
            "wildcard_import" => {
                refs.push(ExtractedRef {
                    source_symbol_index: current_symbol_count,
                    target_name: "*".to_string(),
                    kind: EdgeKind::Imports,
                    line: child.start_position().row as u32,
                    module: module.clone(),
                    chain: None,
                });
            }
            _ => {}
        }
    }
}
