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
                    crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &callee, refs);
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

            // `expr is Type` — emit TypeRef for the checked type.
            "check_expression" => {
                let named: Vec<_> = {
                    let mut nc = child.walk();
                    child.named_children(&mut nc).collect()
                };
                // check_expression: [expression, check_operator, type]
                // The last named child is the type.
                if let Some(type_node) = named.last() {
                    let kind = type_node.kind();
                    if kind != "is_operator" && kind != "is" && kind != "check_operator" {
                        extract_type_ref_from_swift_type(type_node, src, source_symbol_index, refs);
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // `expr as Type` — emit TypeRef for the cast type.
            // In tree-sitter-swift the type is the last named child (after as_operator).
            "as_expression" => {
                // Walk named children: skip the lhs expression and as_operator; the
                // remaining named child is the type node.
                let mut nc = child.walk();
                let named: Vec<_> = child.named_children(&mut nc).collect();
                if let Some(type_node) = named.last() {
                    if type_node.kind() != "as_operator" {
                        extract_type_ref_from_swift_type(type_node, src, source_symbol_index, refs);
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // `{ params in body }` — recurse into lambda/closure body.
            "lambda_literal" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // `"\(expr)"` — recurse into interpolated expressions.
            "line_string_literal" | "multi_line_string_literal" => {
                let mut sc = child.walk();
                for seg in child.children(&mut sc) {
                    if seg.kind() == "interpolated_expression" {
                        extract_calls_from_body(&seg, src, source_symbol_index, refs);
                    }
                }
            }

            // Type references that appear anywhere inside function/closure bodies:
            //   - local variable type annotations  (`let x: MyType = ...`)
            //   - explicit type casts              (`x as! MyType`)
            //   - generic argument lists           (`Array<MyType>`)
            //   - inheritance specifiers on nested types
            "user_type" | "optional_type" | "metatype_type" => {
                extract_type_ref_from_swift_type(&child, src, source_symbol_index, refs);
                // Recurse so generic type arguments inside user_type also emit refs.
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // `SomeProtocol & AnotherProtocol` — emit a ref per component.
            "protocol_composition_type" => {
                extract_protocol_composition_refs(&child, src, source_symbol_index, refs);
            }

            // `var x: MyType` or `let x: MyType` — type annotation inside body.
            "type_annotation" => {
                if let Some(type_node) = child.child_by_field_name("type")
                    .or_else(|| child.named_child(0))
                {
                    if type_node.kind() == "protocol_composition_type" {
                        extract_protocol_composition_refs(&type_node, src, source_symbol_index, refs);
                    } else {
                        extract_type_ref_from_swift_type(&type_node, src, source_symbol_index, refs);
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // `inheritance_specifier` in nested class/struct bodies.
            "inheritance_specifier" | "type_inheritance_clause" => {
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    match inner.kind() {
                        "user_type" | "type_identifier" | "simple_identifier" => {
                            extract_type_ref_from_swift_type(&inner, src, source_symbol_index, refs);
                        }
                        _ => {}
                    }
                }
            }

            _ => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Emit a TypeRef for a Swift type node (user_type, optional_type, array_type, etc.).
pub(super) fn extract_type_ref_from_swift_type(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let name = swift_type_name(node, src);
    if !name.is_empty() {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}

/// Extract the simple name from a Swift type node.
pub(super) fn swift_type_name(node: &Node, src: &[u8]) -> String {
    match node.kind() {
        "user_type" => {
            // user_type → type_identifier+ (e.g. `Array` or `Swift.Array`).
            // Take the last type_identifier.
            let mut last = String::new();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "type_identifier" | "simple_identifier" | "identifier" => {
                        last = node_text(child, src);
                    }
                    _ => {}
                }
            }
            last
        }
        "optional_type" => {
            // Recurse into the wrapped type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let n = swift_type_name(&child, src);
                if !n.is_empty() {
                    return n;
                }
            }
            String::new()
        }
        "array_type" => {
            // `[T]` — the element type.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let n = swift_type_name(&child, src);
                if !n.is_empty() {
                    return n;
                }
            }
            String::new()
        }
        "dictionary_type" => {
            // `[K: V]` — just emit the key type.
            if let Some(key) = node.named_child(0) {
                return swift_type_name(&key, src);
            }
            String::new()
        }
        "function_type" => {
            // `(A) -> B` — emit return type.
            if let Some(ret) = node.child_by_field_name("return_type") {
                return swift_type_name(&ret, src);
            }
            String::new()
        }
        "type_identifier" | "simple_identifier" | "identifier" => node_text(*node, src),
        // `SomeProtocol & AnotherProtocol` — emit the first component
        "protocol_composition_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let n = swift_type_name(&child, src);
                if !n.is_empty() {
                    return n;
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

/// Emit TypeRef edges for ALL type components in a protocol_composition_type.
pub(super) fn extract_protocol_composition_refs(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() != "protocol_composition_type" {
        extract_type_ref_from_swift_type(node, src, source_symbol_index, refs);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let n = swift_type_name(&child, src);
        if !n.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: n,
                kind: crate::types::EdgeKind::TypeRef,
                line: child.start_position().row as u32,
                module: None,
                chain: None,
            });
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
