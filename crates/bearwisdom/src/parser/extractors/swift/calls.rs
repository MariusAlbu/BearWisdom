// =============================================================================
// swift/calls.rs  —  Call extraction and member chain builder for Swift
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
                    let chain = build_chain(callee, src);
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

/// Build a structured member-access chain from a Swift call expression's callee node.
pub(super) fn build_chain(node: Node, src: &[u8]) -> Option<MemberChain> {
    match node.kind() {
        "simple_identifier" | "identifier" => return None,
        _ => {}
    }
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.len() < 2 {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "simple_identifier" | "identifier" | "type_identifier" => {
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

        "self_expression" => {
            segments.push(ChainSegment {
                name: "self".to_string(),
                node_kind: "self_expression".to_string(),
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
            let target = node.child_by_field_name("target")?;
            build_chain_inner(target, src, segments)?;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "navigation_suffix" {
                    let mut nc = child.walk();
                    for inner in child.children(&mut nc) {
                        if inner.kind() == "simple_identifier" {
                            segments.push(ChainSegment {
                                name: node_text(inner, src),
                                node_kind: "simple_identifier".to_string(),
                                kind: SegmentKind::Property,
                                declared_type: None,
                                type_args: vec![],
                                optional_chaining: false,
                            });
                            return Some(());
                        }
                    }
                }
            }
            None
        }

        "call_expression" => {
            let callee = node.named_child(0)?;
            build_chain_inner(callee, src, segments)
        }

        _ => None,
    }
}
