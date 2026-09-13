// =============================================================================
// dart/member_chain.rs  —  the receiver chain of a Dart member access or call
// =============================================================================

use super::helpers::node_text;
use crate::types::{ChainSegment, MemberChain, SegmentKind};
use tree_sitter::Node;

pub(super) fn dart_callee_name(node: Node, src: &str) -> String {
    match node.kind() {
        "identifier" => node_text(node, src),
        "selector_expression" | "navigation_expression" => {
            if let Some(sel) = node.child_by_field_name("selector") {
                return node_text(sel, src);
            }
            let mut last = String::new();
            let mut c = node.walk();
            for n in node.children(&mut c) {
                if n.kind() == "identifier" || n.kind() == "simple_identifier" {
                    last = node_text(n, src);
                }
            }
            last
        }
        _ => {
            let t = node_text(node, src);
            t.rsplit('.').next().unwrap_or(&t).to_string()
        }
    }
}

pub(super) fn build_chain(node: Node, src: &str) -> Option<MemberChain> {
    if node.kind() == "identifier" {
        return None;
    }
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.len() < 2 {
        return None;
    }
    Some(MemberChain { segments })
}

pub(super) fn build_chain_inner(
    node: Node,
    src: &str,
    segments: &mut Vec<ChainSegment>,
) -> Option<()> {
    match node.kind() {
        "identifier" | "simple_identifier" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: node.kind().to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
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
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
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
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            });
            Some(())
        }

        "selector_expression" => {
            let receiver = node
                .child_by_field_name("object")
                .or_else(|| node.named_child(0))?;
            build_chain_inner(receiver, src, segments)?;
            let member_name = node
                .child_by_field_name("selector")
                .map(|n| node_text(n, src))
                .or_else(|| {
                    let mut last: Option<String> = None;
                    let mut c = node.walk();
                    for child in node.children(&mut c) {
                        if child.kind() == "identifier" || child.kind() == "simple_identifier" {
                            last = Some(node_text(child, src));
                        }
                    }
                    last
                })?;
            segments.push(ChainSegment {
                name: member_name,
                node_kind: "selector_expression".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            });
            Some(())
        }

        "navigation_expression" => {
            let receiver = node
                .child_by_field_name("target")
                .or_else(|| node.named_child(0))?;
            build_chain_inner(receiver, src, segments)?;
            let mut last: Option<String> = None;
            let mut c = node.walk();
            for child in node.children(&mut c) {
                if child.kind() == "identifier" || child.kind() == "simple_identifier" {
                    last = Some(node_text(child, src));
                }
            }
            let member_name = last?;
            segments.push(ChainSegment {
                name: member_name,
                node_kind: "navigation_expression".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            });
            Some(())
        }

        "cascade_expression" => {
            let receiver = node.named_child(0)?;
            build_chain_inner(receiver, src, segments)
        }

        "invocation_expression" | "function_invocation" => {
            let callee = node
                .child_by_field_name("function")
                .or_else(|| node.child_by_field_name("name"))
                .or_else(|| node.named_child(0))?;
            build_chain_inner(callee, src, segments)
        }

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Type reference helpers added for coverage gap fixes
// ---------------------------------------------------------------------------
