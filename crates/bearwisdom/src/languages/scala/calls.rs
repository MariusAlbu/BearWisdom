// =============================================================================
// scala/calls.rs  —  Call extraction and member chain builder for Scala
// =============================================================================

use super::decorators::extract_match_patterns;
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
                // call_expression → function (first child), arguments?
                if let Some(callee) = child
                    .child_by_field_name("function")
                    .or_else(|| child.named_child(0))
                {
                    let chain = build_chain(&callee, src);
                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| call_target_name(&callee, src));
                    super::crate::parser::extractors::emit_chain_type_ref(&chain, source_symbol_index, &callee, refs);
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
            // Extract TypeRef edges from `case` patterns in match expressions.
            "match_expression" => {
                extract_match_patterns(&child, src, source_symbol_index, refs);
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // Infix method calls: `a.map(f)` or `list sorted ordering`.
            // The operator field is the method name — emit a Calls edge.
            "infix_expression" => {
                if let Some(op) = child.child_by_field_name("operator") {
                    let target_name = node_text(op, src);
                    if !target_name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: op.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // Lambda expressions: `x => expr`, `(x, y) => expr` — recurse into body.
            "lambda_expression" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            _ => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
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
        "identifier" | "type_identifier" => {
            let name = node_text(*node, src);
            segments.push(ChainSegment {
                name,
                node_kind: "identifier".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "this" => {
            segments.push(ChainSegment {
                name: "this".to_string(),
                node_kind: "this".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "super" => {
            segments.push(ChainSegment {
                name: "super".to_string(),
                node_kind: "super".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "field_expression" | "select_expression" | "field_access" => {
            // field_expression { value: <expr>, field: identifier }
            let value = node.child_by_field_name("value")?;
            let field = node
                .child_by_field_name("field")
                .or_else(|| node.child_by_field_name("name"))?;
            build_chain_inner(&value, src, segments)?;
            segments.push(ChainSegment {
                name: node_text(field, src),
                node_kind: "field_expression".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "call_expression" => {
            // Chained call: function child carries the chain.
            let callee = node
                .child_by_field_name("function")
                .or_else(|| node.named_child(0))?;
            build_chain_inner(&callee, src, segments)
        }

        _ => None,
    }
}
