// =============================================================================
// ruby/calls.rs  —  Call extraction and member chain builder for Ruby
// =============================================================================

use super::helpers::{get_call_method_name, node_text};
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
        if child.kind() == "call" {
            if let Some(mname) = get_call_method_name(&child, src) {
                if mname == "new" {
                    // Emit Instantiates for `ClassName.new`
                    if let Some(recv) = child.child_by_field_name("receiver") {
                        let recv_text = node_text(&recv, src);
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: recv_text,
                            kind: EdgeKind::Instantiates,
                            line: child.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                        // Don't also emit a Calls edge for `.new`.
                        extract_calls_from_body(&child, src, source_symbol_index, refs);
                        continue;
                    }
                }

                let chain = build_chain(&child, src);
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: mname,
                    kind: EdgeKind::Calls,
                    line: child.start_position().row as u32,
                    module: None,
                    chain,
                });
            }
        }
        extract_calls_from_body(&child, src, source_symbol_index, refs);
    }
}

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
        "self" => {
            segments.push(ChainSegment {
                name: "self".to_string(),
                node_kind: "self".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "identifier" | "constant" => {
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

        "call" => {
            // `receiver.method(...)` — recurse into receiver, then push method.
            if let Some(receiver) = node.child_by_field_name("receiver") {
                build_chain_inner(&receiver, src, segments)?;
                if let Some(method) = node.child_by_field_name("method") {
                    segments.push(ChainSegment {
                        name: node_text(&method, src),
                        node_kind: "call".to_string(),
                        kind: SegmentKind::Property,
                        declared_type: None,
                        type_args: vec![],
                        optional_chaining: false,
                    });
                }
                Some(())
            } else {
                // Bare call (no receiver) — treat the method name as Identifier.
                if let Some(method) = node.child_by_field_name("method") {
                    segments.push(ChainSegment {
                        name: node_text(&method, src),
                        node_kind: "call".to_string(),
                        kind: SegmentKind::Identifier,
                        declared_type: None,
                        type_args: vec![],
                        optional_chaining: false,
                    });
                    Some(())
                } else {
                    None
                }
            }
        }

        _ => None,
    }
}
