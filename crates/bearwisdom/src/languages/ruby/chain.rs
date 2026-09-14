// =============================================================================
// ruby/chain.rs — the member chain a Ruby call expression walks
//
// A chain is the receiver path the resolver follows: each segment names one
// hop, the first one naming what the path starts from. An instance variable is
// a member of the object the enclosing method runs on, so `@repo.find` starts
// two hops in — the implicit receiver, then the member — exactly like the
// explicit `self.repo.find`.
// =============================================================================

use super::helpers::node_text;
use crate::types::{ChainSegment, MemberChain, SegmentKind};
use tree_sitter::Node;

#[cfg(test)]
#[path = "chain_tests.rs"]
mod tests;

pub(super) fn build_chain(node: &Node, src: &[u8]) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

/// A segment carrying only a name, a hop kind and a source address. Ruby
/// states no type arguments, optional chaining, or call arguments on a chain
/// hop. A `byte_offset` of 0 leaves the address to the position pass, which
/// locates each segment's name in the source text; a hop whose name is not
/// written at its own position states its address here instead.
fn segment(name: String, node_kind: &str, kind: SegmentKind, byte_offset: u32) -> ChainSegment {
    ChainSegment {
        name,
        node_kind: node_kind.to_string(),
        kind,
        declared_type: None,
        type_args: vec![],
        optional_chaining: false,
        byte_offset,
        declared_type_id: None,
        is_call: false,
        call_args: Vec::new(),
        type_arg_ids: Vec::new(),
    }
}

fn build_chain_inner(node: &Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "self" => {
            segments.push(segment("self".to_string(), "self", SegmentKind::SelfRef, 0));
            Some(())
        }

        // `@name` — the implicit receiver's member. The sigil is part of the
        // member's name: Ruby reads and writes `@name` under that spelling,
        // while `name` is the reader method an `attr_*` macro declares. The
        // receiver is written nowhere, so the two hops address the sigil and
        // the name it introduces.
        "instance_variable" => {
            let at = node.start_byte() as u32;
            segments.push(segment(
                "self".to_string(),
                "self",
                SegmentKind::SelfRef,
                at,
            ));
            segments.push(segment(
                node_text(node, src),
                "instance_variable",
                SegmentKind::Property,
                at.saturating_add(1),
            ));
            Some(())
        }

        "identifier" | "constant" => {
            segments.push(segment(
                node_text(node, src),
                node.kind(),
                SegmentKind::Identifier,
                0,
            ));
            Some(())
        }

        "call" => {
            // `receiver.method(...)` — recurse into receiver, then push method.
            if let Some(receiver) = node.child_by_field_name("receiver") {
                build_chain_inner(&receiver, src, segments)?;
                if let Some(method) = node.child_by_field_name("method") {
                    segments.push(segment(
                        node_text(&method, src),
                        "call",
                        SegmentKind::Property,
                        0,
                    ));
                }
                Some(())
            } else {
                // Bare call (no receiver) — treat the method name as Identifier.
                let method = node.child_by_field_name("method")?;
                segments.push(segment(
                    node_text(&method, src),
                    "call",
                    SegmentKind::Identifier,
                    0,
                ));
                Some(())
            }
        }

        _ => None,
    }
}
