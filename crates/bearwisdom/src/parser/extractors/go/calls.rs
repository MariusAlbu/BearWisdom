// =============================================================================
// go/calls.rs  —  Call and reference extraction for Go
// =============================================================================

use super::helpers::node_text;
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Body reference extraction (calls, instantiations)
// ---------------------------------------------------------------------------

pub(super) fn extract_refs_from_body(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call_expression" => {
                extract_call_ref(&child, source, source_symbol_index, refs);
                // Recurse into arguments for nested calls.
                let mut acursor = child.walk();
                for arg_child in child.children(&mut acursor) {
                    if arg_child.kind() == "argument_list" {
                        extract_refs_from_body(
                            &arg_child,
                            source,
                            source_symbol_index,
                            refs,
                        );
                    }
                }
            }
            "composite_literal" => {
                extract_composite_literal_ref(&child, source, source_symbol_index, refs);
                // Recurse into body for nested composites / calls.
                let mut bcursor = child.walk();
                for body_child in child.children(&mut bcursor) {
                    if body_child.kind() == "literal_value" {
                        extract_refs_from_body(
                            &body_child,
                            source,
                            source_symbol_index,
                            refs,
                        );
                    }
                }
            }

            // `x.(*Admin)` — type assertion
            "type_assertion_expression" => {
                extract_type_assertion_ref(&child, source, source_symbol_index, refs);
                extract_refs_from_body(&child, source, source_symbol_index, refs);
            }

            // `switch v := x.(type) { case *Admin: ... }`
            "type_switch_statement" => {
                extract_type_switch_refs(&child, source, source_symbol_index, refs);
                extract_refs_from_body(&child, source, source_symbol_index, refs);
            }

            _ => {
                extract_refs_from_body(&child, source, source_symbol_index, refs);
            }
        }
    }
}

/// Emit a `Calls` ref for a `call_expression`.
///
/// `call_expression` children (positional):
///   function (identifier | selector_expression | ...), argument_list
///
/// For `bar.Baz()` the function part is a `selector_expression` with children:
///   operand, `.`, `field_identifier`
fn extract_call_ref(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The function part is the first named child (use index to avoid cursor borrow).
    let func_node = match node.named_child(0) {
        Some(n) => n,
        None => return,
    };

    // Build a structured chain for selector expressions; fall back to the
    // existing single-name extraction for bare identifiers.
    let chain = build_chain(func_node, source);

    let target_name = chain
        .as_ref()
        .and_then(|c| c.segments.last())
        .map(|s| s.name.clone())
        .unwrap_or_else(|| match func_node.kind() {
            "selector_expression" => (0..func_node.named_child_count())
                .filter_map(|i| func_node.named_child(i))
                .find(|c| c.kind() == "field_identifier")
                .map(|n| node_text(&n, source))
                .unwrap_or_else(|| node_text(&func_node, source)),
            _ => node_text(&func_node, source),
        });

    if target_name.is_empty() {
        return;
    }

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name,
        kind: EdgeKind::Calls,
        line: func_node.start_position().row as u32,
        module: None,
        chain,
    });
}

// ---------------------------------------------------------------------------
// Member chain builder
// ---------------------------------------------------------------------------

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

/// Emit an `Instantiates` ref for a `composite_literal`.
///
/// `composite_literal` children: type (identifier or qualified_type), literal_value
pub(super) fn extract_composite_literal_ref(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The type is the first named child (use index to avoid cursor borrow).
    let type_node = match node.named_child(0) {
        Some(n) => n,
        None => return,
    };

    // Skip if the first named child is the literal_value `{...}` (happens for
    // anonymous composite literals like `{1, 2}`).
    if type_node.kind() == "literal_value" {
        return;
    }

    let type_name = match type_node.kind() {
        "type_identifier" => node_text(&type_node, source),
        "qualified_type" => {
            // `pkg.TypeName` — find the last `type_identifier` by index.
            let last_ti = (0..type_node.named_child_count())
                .filter_map(|i| type_node.named_child(i))
                .filter(|c| c.kind() == "type_identifier")
                .last();
            match last_ti {
                Some(n) => node_text(&n, source),
                None => node_text(&type_node, source),
            }
        }
        _ => node_text(&type_node, source),
    };

    if type_name.is_empty() {
        return;
    }

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: type_name,
        kind: EdgeKind::Instantiates,
        line: type_node.start_position().row as u32,
        module: None,
        chain: None,
    });
}

// ---------------------------------------------------------------------------
// Type narrowing — type assertions and type switches
// ---------------------------------------------------------------------------

/// Emit a TypeRef for `x.(*Admin)` — a `type_assertion_expression`.
///
/// Tree-sitter-go structure:
/// ```text
/// type_assertion_expression
///   identifier "x"          ← operand
///   pointer_type / type_identifier / qualified_type   ← asserted type
/// ```
/// The asserted type is the last named child.
pub(super) fn extract_type_assertion_ref(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let named_count = node.named_child_count();
    if named_count < 2 {
        return;
    }
    let type_node = match node.named_child(named_count - 1) {
        Some(n) => n,
        None => return,
    };

    let type_name = go_type_node_name(&type_node, source);
    if type_name.is_empty() {
        return;
    }

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: type_name,
        kind: EdgeKind::TypeRef,
        line: type_node.start_position().row as u32,
        module: None,
        chain: None,
    });
}

/// Emit TypeRefs for each case type in a `type_switch_statement`.
///
/// ```go
/// switch v := x.(type) {
///     case *Admin:   ...
///     case *User:    ...
/// }
/// ```
/// Tree-sitter-go: `type_switch_statement` → `type_case` children,
/// each with a `type` field (or positional type children).
pub(super) fn extract_type_switch_refs(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_case" {
            // Each case clause can list multiple types: `case *Foo, *Bar:`
            // Walk all children for type nodes.
            let mut inner = child.walk();
            for type_child in child.children(&mut inner) {
                match type_child.kind() {
                    "type_identifier" | "pointer_type" | "qualified_type" => {
                        let name = go_type_node_name(&type_child, source);
                        if !name.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::TypeRef,
                                line: type_child.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Extract a simple type name from a Go type node, dereferencing pointer types.
fn go_type_node_name(node: &Node, source: &str) -> String {
    match node.kind() {
        "type_identifier" => node_text(node, source),
        "pointer_type" => {
            // `*Admin` — the named child is the underlying type.
            node.named_child(0)
                .map(|n| go_type_node_name(&n, source))
                .unwrap_or_default()
        }
        "qualified_type" => {
            // `pkg.Admin` — use the last type_identifier.
            (0..node.named_child_count())
                .filter_map(|i| node.named_child(i))
                .filter(|c| c.kind() == "type_identifier")
                .last()
                .map(|n| node_text(&n, source))
                .unwrap_or_else(|| node_text(node, source))
        }
        _ => String::new(),
    }
}
