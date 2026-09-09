//! Source call-chain shape; semantic binding remains in the generic engine.
use super::{node_text, rust_type_node_name};
use crate::types::{ChainSegment, MemberChain, SegmentKind};
use tree_sitter::Node;

/// Build a structured member-access chain from a Rust call expression's function node.
///
/// Returns `None` for bare single-segment identifiers.
pub(super) fn build_chain(node: Node, source: &str) -> Option<MemberChain> {
    if node.kind() == "identifier" || node.kind() == "self" {
        return None;
    }
    let mut segments = Vec::new();
    build_chain_inner(node, source, &mut segments)?;
    if segments.len() < 2
        && !super::super::namespaces::FORMS
            .types
            .call_applications
            .iter()
            .any(|&(kind, _, _)| kind == node.kind())
    {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: Node, source: &str, segments: &mut Vec<ChainSegment>) -> Option<()> {
    if let Some(&(_, head, arguments)) = super::super::namespaces::FORMS
        .types
        .call_applications
        .iter()
        .find(|&&(kind, _, _)| kind == node.kind())
    {
        build_chain_inner(node.child_by_field_name(head)?, source, segments)?;
        let args = node.child_by_field_name(arguments)?;
        let mut cursor = args.walk();
        segments.last_mut()?.type_args = args
            .named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .map(|n| node_text(&n, source))
            .collect();
        return Some(());
    }
    match node.kind() {
        "identifier" | "type_identifier" | "crate" | "super" | "bracketed_type" => {
            segments.push(ChainSegment {
                name: node_text(&node, source),
                node_kind: if node.kind() != "bracketed_type"
                    && node.parent().is_some_and(|p| {
                        matches!(p.kind(), "scoped_identifier" | "scoped_type_identifier")
                    }) {
                    "scoped_identifier"
                } else {
                    node.kind()
                }
                .to_string(),
                kind: if node_text(&node, source) == "Self" {
                    SegmentKind::SelfRef
                } else {
                    SegmentKind::Identifier
                },
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

        "self" => {
            segments.push(ChainSegment {
                name: "self".to_string(),
                node_kind: "self".to_string(),
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

        "field_expression" => {
            let value = node.child_by_field_name("value")?;
            let field = node.child_by_field_name("field")?;
            build_chain_inner(value, source, segments)?;
            segments.push(ChainSegment {
                name: node_text(&field, source),
                node_kind: field.kind().to_string(),
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

        "scoped_identifier" | "scoped_type_identifier" => {
            // CST ownership, never split a qualified type/generic argument's
            // display text into fictitious value-chain segments.
            if let Some(path) = node.child_by_field_name("path") {
                build_chain_inner(path, source, segments)?;
            }
            let name = node.child_by_field_name("name")?;
            let kind = if segments.is_empty() {
                SegmentKind::Identifier
            } else {
                SegmentKind::Property
            };
            segments.push(ChainSegment {
                name: node_text(&name, source),
                node_kind: "scoped_identifier".to_string(),
                kind,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
                byte_offset: name.start_byte() as u32,
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
            build_chain_inner(func, source, segments)?;
            if let Some(last) = segments.last_mut() {
                last.is_call = true;
            }
            Some(())
        }

        // `container[index]` — a subscript. The grammar carries no field
        // names for `index_expression`; the container is the first named
        // child, the index expression the second. Recurse into the
        // container, then push a ComputedAccess segment carrying the index
        // text — the chain walker projects the container's element type at
        // this segment (`array_element_type` in engine/chain.rs) rather than
        // looking up a member literally named by the index.
        "index_expression" => {
            let container = node.named_child(0)?;
            build_chain_inner(container, source, segments)?;
            let index_text = node
                .named_child(1)
                .map(|n| node_text(&n, source))
                .unwrap_or_default();
            segments.push(ChainSegment {
                name: index_text,
                node_kind: "index_expression".to_string(),
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

        // `(x as Foo).bar()` — `x` is the value, `Foo` the asserted type. The
        // chain walker adopts the inner segment's `declared_type`.
        "type_cast_expression" => {
            let value = node.child_by_field_name("value")?;
            build_chain_inner(value, source, segments)?;
            if let Some(type_node) = node.child_by_field_name("type") {
                let name = rust_type_node_name(&type_node, source);
                if !name.is_empty() {
                    if let Some(last) = segments.last_mut() {
                        if last.declared_type.is_none() {
                            last.declared_type = Some(name);
                        }
                    }
                }
            }
            Some(())
        }

        _ => None,
    }
}

#[cfg(test)]
#[path = "calls_chain_tests.rs"]
mod tests;
