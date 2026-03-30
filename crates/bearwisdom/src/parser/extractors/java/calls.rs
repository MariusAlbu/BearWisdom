// =============================================================================
// java/calls.rs  —  Call extraction and member chain building for Java
// =============================================================================

use super::helpers::{node_text, type_node_simple_name};
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
            "method_invocation" => {
                // `name` field is always present (identifier).
                if let Some(name_node) = child.child_by_field_name("name") {
                    let chain = build_chain(&child, src);
                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| node_text(name_node, src));
                    if !target_name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: name_node.start_position().row as u32,
                            module: None,
                            chain,
                        });
                    }
                }
                // Recurse into arguments — nested calls.
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            "object_creation_expression" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    let name = type_node_simple_name(type_node, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Instantiates,
                            line: type_node.start_position().row as u32,
                            module: None,
                            chain: None,
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

// ---------------------------------------------------------------------------
// Member chain builder
// ---------------------------------------------------------------------------

/// Build a structured member access chain from a Java CST node.
///
/// Java uses `method_invocation` for calls and `field_access` for member reads:
///
/// ```text
/// method_invocation
///   object: method_invocation      ← chained call
///     object: identifier "this"
///     name:   identifier "getRepo"
///   name: identifier "findOne"
/// ```
/// produces: `[this, getRepo, findOne]`
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
        "identifier" => {
            let name = node_text(*node, src);
            let kind = SegmentKind::Identifier;
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

        "method_invocation" => {
            // method_invocation { object?: <expr>, name: identifier }
            if let Some(object) = node.child_by_field_name("object") {
                build_chain_inner(&object, src, segments)?;
            }
            let name_node = node.child_by_field_name("name")?;
            segments.push(ChainSegment {
                name: node_text(name_node, src),
                node_kind: "method_invocation".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "field_access" => {
            // field_access { object: <expr>, field: identifier }
            let object = node.child_by_field_name("object")?;
            let field = node.child_by_field_name("field")?;
            build_chain_inner(&object, src, segments)?;
            segments.push(ChainSegment {
                name: node_text(field, src),
                node_kind: "field_access".to_string(),
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
