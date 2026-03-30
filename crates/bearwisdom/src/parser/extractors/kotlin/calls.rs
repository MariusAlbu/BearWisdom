// =============================================================================
// kotlin/calls.rs  —  Call extraction and member chain builder for Kotlin
// =============================================================================

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
            _ => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
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
