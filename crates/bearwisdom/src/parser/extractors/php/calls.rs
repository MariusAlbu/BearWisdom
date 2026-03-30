// =============================================================================
// php/calls.rs  —  Call extraction and import ref helpers for PHP
// =============================================================================

use super::helpers::node_text;
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

pub(super) fn extract_calls_from_body(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "member_call_expression" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let callee = node_text(&name_node, src);
                    let chain = build_chain(&child, src);
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: callee,
                        kind: EdgeKind::Calls,
                        line: name_node.start_position().row as u32,
                        module: None,
                        chain,
                    });
                }
            }

            "static_call_expression" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let callee = node_text(&name_node, src);
                    let chain = build_chain(&child, src);
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: callee,
                        kind: EdgeKind::Calls,
                        line: name_node.start_position().row as u32,
                        module: None,
                        chain,
                    });
                }
            }

            "object_creation_expression" => {
                let cls_node_opt = if let Some(n) = child.child_by_field_name("class_type") {
                    Some(n)
                } else {
                    let mut c = child.walk();
                    let mut found = None;
                    for n in child.children(&mut c) {
                        if n.kind() == "name"
                            || n.kind() == "qualified_name"
                            || n.kind() == "identifier"
                            || n.kind() == "variable_name"
                        {
                            found = Some(n);
                            break;
                        }
                    }
                    found
                };
                if let Some(cls_node) = cls_node_opt {
                    let cls_name = node_text(&cls_node, src);
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: cls_name,
                        kind: EdgeKind::Instantiates,
                        line: cls_node.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }

            "function_call_expression" => {
                if let Some(fn_node) = child.child_by_field_name("function") {
                    let callee = node_text(&fn_node, src);
                    let simple = callee.rsplit('\\').next().unwrap_or(&callee).to_string();
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: simple,
                        kind: EdgeKind::Calls,
                        line: fn_node.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }

            _ => {}
        }
        extract_calls_from_body(&child, src, source_symbol_index, refs);
    }
}

// ---------------------------------------------------------------------------
// Member chain builder
// ---------------------------------------------------------------------------

pub(super) fn build_chain(node: &Node, src: &[u8]) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: &Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "variable_name" => {
            let raw = node_text(node, src);
            let name = raw.trim_start_matches('$').to_string();
            let kind = if name == "this" {
                SegmentKind::SelfRef
            } else {
                SegmentKind::Identifier
            };
            segments.push(ChainSegment {
                name,
                node_kind: "variable_name".to_string(),
                kind,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "name" | "identifier" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: node.kind().to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "member_access_expression" => {
            let object = node.child_by_field_name("object")?;
            let name_node = node.child_by_field_name("name")?;
            build_chain_inner(&object, src, segments)?;
            segments.push(ChainSegment {
                name: node_text(&name_node, src),
                node_kind: "member_access_expression".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "member_call_expression" => {
            let object = node.child_by_field_name("object")?;
            let name_node = node.child_by_field_name("name")?;
            build_chain_inner(&object, src, segments)?;
            segments.push(ChainSegment {
                name: node_text(&name_node, src),
                node_kind: "member_call_expression".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "static_call_expression" | "scoped_call_expression" => {
            let class_node = node.child_by_field_name("class")?;
            let name_node = node.child_by_field_name("name")?;
            segments.push(ChainSegment {
                name: node_text(&class_node, src),
                node_kind: "class".to_string(),
                kind: SegmentKind::TypeAccess,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            segments.push(ChainSegment {
                name: node_text(&name_node, src),
                node_kind: "static_call_expression".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Use declaration / import reference extraction
// ---------------------------------------------------------------------------

pub(super) fn extract_use_declaration(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace_use_clause" => {
                push_use_ref_for_name(&child, src, refs, current_symbol_count);
            }
            "qualified_name" | "name" => {
                let full = node_text(&child, src);
                push_fq_import(full, child.start_position().row as u32, refs, current_symbol_count);
            }
            _ => {}
        }
    }
}

fn push_use_ref_for_name(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "qualified_name" || child.kind() == "name" {
            let full = node_text(&child, src);
            push_fq_import(full, child.start_position().row as u32, refs, current_symbol_count);
            return;
        }
    }
}

/// Push an Imports edge for a fully-qualified PHP name like `Foo\Bar\Baz`.
fn push_fq_import(
    full: String,
    line: u32,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let parts: Vec<&str> = full.split('\\').collect();
    let target = parts.last().unwrap_or(&full.as_str()).to_string();
    let module = if parts.len() > 1 {
        Some(parts[..parts.len() - 1].join("\\"))
    } else {
        None
    };
    refs.push(ExtractedRef {
        source_symbol_index: current_symbol_count,
        target_name: target,
        kind: EdgeKind::Imports,
        line,
        module,
        chain: None,
    });
}

pub(super) fn extract_trait_use(
    node: &Node,
    src: &[u8],
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "qualified_name" || child.kind() == "name" {
            let full = node_text(&child, src);
            push_fq_import(full, child.start_position().row as u32, refs, current_symbol_count);
        }
    }
}
