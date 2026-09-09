//! JavaScript grammar chain extraction; receiver kinds come from profile data.
use super::super::common::node_text_bytes;
use crate::types::{ChainSegment, MemberChain, SegmentKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Shared chain builder — language-agnostic, works for any grammar that uses
// the standard tree-sitter JS/TS node kinds (member_expression, identifier,
// call_expression, subscript_expression, this, super).
// ---------------------------------------------------------------------------

/// Build a structured member-access chain from a tree-sitter function node.
///
/// Returns `None` when the node isn't a recognisable chain root (e.g. an
/// anonymous arrow function as the callee, which can't be named).
///
/// Works with both the TypeScript and JavaScript grammars — both grammars
/// share the same node kinds for all patterns covered here.
pub fn build_member_chain(node: Node, src: &[u8]) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "this" | "super" => {
            segments.push(ChainSegment {
                name: node_text_bytes(node, src),
                node_kind: node.kind().to_string(),
                kind: super::profile::RECEIVER_NODES
                    .iter()
                    .find(|(kind, _)| *kind == node.kind())?
                    .1,
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

        "identifier" | "property_identifier" => {
            segments.push(ChainSegment {
                name: node_text_bytes(node, src),
                node_kind: "identifier".to_string(),
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

        "member_expression" => {
            let object = node.child_by_field_name("object")?;
            let property = node.child_by_field_name("property")?;

            let is_optional = (0..node.child_count()).any(|i| {
                node.child(i)
                    .map(|c| c.kind() == "optional_chain")
                    .unwrap_or(false)
            });

            build_chain_inner(object, src, segments)?;

            segments.push(ChainSegment {
                name: node_text_bytes(property, src),
                node_kind: property.kind().to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: is_optional,
                byte_offset: 0,
                declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
            });
            Some(())
        }

        "subscript_expression" => {
            let object = node.child_by_field_name("object")?;
            let index = node.child_by_field_name("index")?;

            build_chain_inner(object, src, segments)?;

            segments.push(ChainSegment {
                name: node_text_bytes(index, src),
                node_kind: "subscript_expression".to_string(),
                kind: SegmentKind::ComputedAccess,
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

        "call_expression" => {
            // Nested call in a chain: `a.b().c()` — walk into the function child,
            // then mark the resolved segment as invoked so the walker yields the
            // function's return type rather than the function value itself.
            let func = node.child_by_field_name("function")?;
            build_chain_inner(func, src, segments)?;
            if let Some(last) = segments.last_mut() {
                last.is_call = true;
            }
            Some(())
        }

        // Non-chainable node (arrow, conditional, etc.) — abort.
        _ => None,
    }
}

#[cfg(test)]
#[path = "chains_tests.rs"]
mod tests;
