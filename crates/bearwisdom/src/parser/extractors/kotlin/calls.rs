// =============================================================================
// kotlin/calls.rs  —  Call extraction and member chain builder for Kotlin
// =============================================================================

use super::decorators::extract_when_patterns;
use super::helpers::{call_target_name, node_text};
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

pub(super) fn extract_calls_from_body(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call_expression" => {
                if let Some(callee) = child.named_child(0) {
                    let chain = build_chain(&callee, src);
                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| call_target_name(&callee, src));
                    super::super::emit_chain_type_ref(&chain, source_symbol_index, &callee, refs);
                    if !target_name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: callee.start_position().row as u32,
                            module: None,
                            chain,
                        });
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // Extract TypeRef edges from `is` checks inside when expressions.
            "when_expression" => {
                extract_when_patterns(&child, src, source_symbol_index, refs);
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `x as Type` — emit TypeRef for the cast type (field: right).
            "as_expression" => {
                if let Some(type_node) = child.child_by_field_name("right") {
                    extract_type_ref_from_type_node(&type_node, src, source_symbol_index, refs);
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `x is Type` — emit TypeRef for the checked type (field: right).
            "is_expression" => {
                if let Some(type_node) = child.child_by_field_name("right") {
                    extract_type_ref_from_type_node(&type_node, src, source_symbol_index, refs);
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `"Hello ${expr}"` — recurse into interpolated expressions.
            "string_literal" => {
                let mut sc = child.walk();
                for seg in child.children(&mut sc) {
                    if seg.kind() == "interpolation" {
                        extract_calls_from_body(&seg, src, source_symbol_index, refs);
                    }
                }
            }
            _ => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Emit a TypeRef edge for a Kotlin `type` or `user_type` node.
pub(super) fn extract_type_ref_from_type_node(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = kotlin_type_name(node, src);
    if !name.is_empty() {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}

/// Extract the simple name from a Kotlin type node.
pub(super) fn kotlin_type_name(node: &Node, src: &[u8]) -> String {
    match node.kind() {
        "user_type" => {
            // user_type → simple_user_type+ — take the last segment's identifier.
            let mut last = String::new();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "simple_user_type" {
                    let mut ic = child.walk();
                    for inner in child.children(&mut ic) {
                        if inner.kind() == "simple_identifier" || inner.kind() == "identifier" {
                            last = node_text(inner, src);
                            break;
                        }
                    }
                } else if child.kind() == "simple_identifier" || child.kind() == "identifier" {
                    last = node_text(child, src);
                }
            }
            last
        }
        "type" | "nullable_type" => {
            // Recurse into the inner type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let name = kotlin_type_name(&child, src);
                if !name.is_empty() {
                    return name;
                }
            }
            String::new()
        }
        "simple_identifier" | "identifier" | "type_identifier" => node_text(*node, src),
        _ => String::new(),
    }
}


/// Build a structured member access chain from a Kotlin CST node.
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
        "simple_identifier" | "identifier" => {
            let name = node_text(*node, src);
            segments.push(ChainSegment {
                name,
                node_kind: "simple_identifier".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "this_expression" => {
            segments.push(ChainSegment {
                name: "this".to_string(),
                node_kind: "this_expression".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "super_expression" => {
            segments.push(ChainSegment {
                name: "super".to_string(),
                node_kind: "super_expression".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "navigation_expression" => {
            let mut cursor = node.walk();
            let mut children = node.children(&mut cursor);
            let receiver = children.find(|c| c.is_named())?;
            build_chain_inner(&receiver, src, segments)?;
            let mut cursor2 = node.walk();
            for child in node.children(&mut cursor2) {
                if child.kind() == "navigation_suffix" {
                    let mut nc = child.walk();
                    for inner in child.children(&mut nc) {
                        if inner.kind() == "simple_identifier" {
                            segments.push(ChainSegment {
                                name: node_text(inner, src),
                                node_kind: "navigation_suffix".to_string(),
                                kind: SegmentKind::Property,
                                declared_type: None,
                                type_args: vec![],
                                optional_chaining: false,
                            });
                            break;
                        }
                    }
                }
            }
            Some(())
        }

        "call_expression" => {
            let callee = node.named_child(0)?;
            build_chain_inner(&callee, src, segments)
        }

        _ => None,
    }
}
