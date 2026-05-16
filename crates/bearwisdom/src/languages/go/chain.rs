// =============================================================================
// go/chain.rs  —  Member chain builder for Go selector expressions
// =============================================================================

use super::helpers::node_text;
use crate::types::{ChainSegment, MemberChain, SegmentKind};
use tree_sitter::Node;

/// Build a structured `MemberChain` from a Go function/selector node.
///
/// Go uses `selector_expression` for member access (not `member_expression`):
///
/// `repo.FindOne()`:
/// ```text
/// selector_expression
///   identifier "repo"
///   field_identifier "FindOne"
/// ```
///
/// `s.repo.FindOne()`:
/// ```text
/// selector_expression
///   selector_expression
///     identifier "s"
///     field_identifier "repo"
///   field_identifier "FindOne"
/// ```
///
/// Returns `None` for bare `identifier` nodes (single-segment — handled by
/// the existing scope-chain strategies) and for any node we can't walk.
pub(super) fn build_chain(node: Node, source: &str) -> Option<MemberChain> {
    // Only build a chain for multi-segment expressions.
    if node.kind() == "identifier" {
        return None;
    }
    let mut segments = Vec::new();
    build_chain_inner(node, source, &mut segments)?;
    if segments.len() < 2 {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: Node, source: &str, segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "identifier" => {
            segments.push(ChainSegment {
                name: node_text(&node, source),
                node_kind: "identifier".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "selector_expression" => {
            // Children (by index): operand, `.` (anon), field_identifier
            // We need the first named child (operand) and the last named child
            // (field_identifier).  Use indexed access to avoid cursor re-borrow.
            let named_count = node.named_child_count();
            if named_count < 2 {
                return None;
            }
            let operand = node.named_child(0)?;
            let field = node.named_child(named_count - 1)?;

            // Recurse into the operand to build the prefix chain.
            build_chain_inner(operand, source, segments)?;

            segments.push(ChainSegment {
                name: node_text(&field, source),
                node_kind: field.kind().to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "call_expression" => {
            // Nested call in a chain: `a.B().C()` — walk into its function child.
            let func = node.named_child(0)?;
            build_chain_inner(func, source, segments)
        }

        // Unknown node — can't build a chain from this.
        _ => None,
    }
}
