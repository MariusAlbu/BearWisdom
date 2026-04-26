// =============================================================================
// c_lang/calls.rs  —  Call extraction and member chain builder for C/C++
// =============================================================================

use super::helpers::{call_target_name, node_text};
use super::symbols::emit_typerefs_for_type_descriptor;
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
            // -----------------------------------------------------------------
            // Calls
            // -----------------------------------------------------------------
            "call_expression" => {
                if let Some(fn_node) = child.child_by_field_name("function") {
                    let chain = build_chain(fn_node, src);
                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| call_target_name(&fn_node, src));
                    crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &fn_node, refs);
                    if !target_name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: fn_node.start_position().row as u32,
                            module: None,
                            chain,
                            byte_offset: fn_node.start_byte() as u32,
                                                    namespace_segments: Vec::new(),
});
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // -----------------------------------------------------------------
            // C-style cast: `(Type)value`
            // -----------------------------------------------------------------
            "cast_expression" => {
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    if inner.kind() == "type_descriptor" {
                        emit_typerefs_for_type_descriptor(inner, src, source_symbol_index, refs);
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // -----------------------------------------------------------------
            // sizeof(Type) or sizeof expr
            // sizeof(Foo) parses as sizeof + parenthesized_expression → identifier
            // sizeof(struct Foo) parses as sizeof + type_descriptor
            // -----------------------------------------------------------------
            "sizeof_expression" => {
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    match inner.kind() {
                        "type_descriptor" => {
                            emit_typerefs_for_type_descriptor(inner, src, source_symbol_index, refs);
                        }
                        "parenthesized_expression" => {
                            // `sizeof(Foo)` where Foo looks like a value expression.
                            // Emit TypeRef for any bare identifier inside.
                            let mut pc = inner.walk();
                            for pchild in inner.children(&mut pc) {
                                if pchild.kind() == "identifier" || pchild.kind() == "type_identifier" {
                                    let name = node_text(pchild, src);
                                    if !name.is_empty() {
                                        refs.push(ExtractedRef {
                                            source_symbol_index,
                                            target_name: name,
                                            kind: EdgeKind::TypeRef,
                                            line: pchild.start_position().row as u32,
                                            module: None,
                                            chain: None,
                                            byte_offset: 0,
                                                                                    namespace_segments: Vec::new(),
});
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // -----------------------------------------------------------------
            // new Foo() / new Foo[]
            // -----------------------------------------------------------------
            "new_expression" => {
                // The constructed type may be a `type_identifier` or a
                // `template_type` direct child (no `type_descriptor` wrapper).
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    match inner.kind() {
                        "type_identifier" => {
                            let name = node_text(inner, src);
                            if !name.is_empty() {
                                refs.push(ExtractedRef {
                                    source_symbol_index,
                                    target_name: name.clone(),
                                    kind: EdgeKind::Instantiates,
                                    line: inner.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
});
                                refs.push(ExtractedRef {
                                    source_symbol_index,
                                    target_name: name,
                                    kind: EdgeKind::TypeRef,
                                    line: inner.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
});
                            }
                        }
                        "template_type" => {
                            if let Some(name_node) = inner.child(0) {
                                if name_node.kind() == "type_identifier" {
                                    let name = node_text(name_node, src);
                                    if !name.is_empty() {
                                        refs.push(ExtractedRef {
                                            source_symbol_index,
                                            target_name: name.clone(),
                                            kind: EdgeKind::Instantiates,
                                            line: name_node.start_position().row as u32,
                                            module: None,
                                            chain: None,
                                            byte_offset: 0,
                                                                                    namespace_segments: Vec::new(),
});
                                        refs.push(ExtractedRef {
                                            source_symbol_index,
                                            target_name: name,
                                            kind: EdgeKind::TypeRef,
                                            line: name_node.start_position().row as u32,
                                            module: None,
                                            chain: None,
                                            byte_offset: 0,
                                                                                    namespace_segments: Vec::new(),
});
                                    }
                                }
                            }
                            // also recurse into template args for TypeRefs
                            emit_typerefs_for_type_descriptor(inner, src, source_symbol_index, refs);
                        }
                        _ => {}
                    }
                }
                // recurse for calls in argument list
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // -----------------------------------------------------------------
            // lambda_expression — recurse into body for calls; param types as TypeRef
            // -----------------------------------------------------------------
            "lambda_expression" => {
                // Extract TypeRef from parameter types in the abstract_function_declarator.
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    if inner.kind() == "abstract_function_declarator" {
                        extract_lambda_param_typerefs(&inner, src, source_symbol_index, refs);
                    }
                }
                // Recurse into body for calls.
                if let Some(body) = child.child_by_field_name("body") {
                    extract_calls_from_body(&body, src, source_symbol_index, refs);
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }

            // -----------------------------------------------------------------
            // try/catch — catch_clause: TypeRef for exception type + Variable
            // -----------------------------------------------------------------
            "try_statement" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            "catch_clause" => {
                if let Some(params) = child.child_by_field_name("parameters") {
                    extract_catch_typerefs(&params, src, source_symbol_index, refs);
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // -----------------------------------------------------------------
            // template_type in expressions (e.g. `vector<int> v;` in body)
            // -----------------------------------------------------------------
            "template_type" => {
                emit_typerefs_for_type_descriptor(child, src, source_symbol_index, refs);
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            _ => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Extract TypeRef from lambda parameter types.
fn extract_lambda_param_typerefs(
    decl: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = decl.walk();
    for child in decl.children(&mut cursor) {
        if child.kind() == "parameter_list" {
            let mut pc = child.walk();
            for param in child.children(&mut pc) {
                if param.kind() == "parameter_declaration" {
                    if let Some(type_node) = param.child_by_field_name("type") {
                        emit_typerefs_for_type_descriptor(type_node, src, source_symbol_index, refs);
                    } else {
                        // Fallback: walk for type_identifier children.
                        let mut ic = param.walk();
                        for inner in param.children(&mut ic) {
                            if inner.kind() == "type_identifier" {
                                let name = node_text(inner, src);
                                if !name.is_empty() {
                                    refs.push(ExtractedRef {
                                        source_symbol_index,
                                        target_name: name,
                                        kind: EdgeKind::TypeRef,
                                        line: inner.start_position().row as u32,
                                        module: None,
                                        chain: None,
                                        byte_offset: 0,
                                                                            namespace_segments: Vec::new(),
});
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Emit TypeRef for exception types in catch parameter lists.
fn extract_catch_typerefs(
    params: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = params.walk();
    for child in params.children(&mut cursor) {
        if child.kind() == "parameter_declaration" {
            // The type is the first named child (qualified_identifier or type_identifier).
            let mut ic = child.walk();
            for inner in child.children(&mut ic) {
                match inner.kind() {
                    "type_identifier" => {
                        let name = node_text(inner, src);
                        if !name.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::TypeRef,
                                line: inner.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                                                            namespace_segments: Vec::new(),
});
                        }
                    }
                    "qualified_identifier" => {
                        // `std::exception` → emit the last name component.
                        if let Some(name_node) = inner.child_by_field_name("name") {
                            let name = node_text(name_node, src);
                            if !name.is_empty() {
                                refs.push(ExtractedRef {
                                    source_symbol_index,
                                    target_name: name,
                                    kind: EdgeKind::TypeRef,
                                    line: name_node.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
});
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

pub(super) fn build_chain(node: Node, src: &[u8]) -> Option<MemberChain> {
    match node.kind() {
        "identifier" | "field_identifier" => return None,
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
        "identifier" | "field_identifier" | "type_identifier" => {
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

        "field_expression" => {
            let argument = node.child_by_field_name("argument")?;
            let field = node.child_by_field_name("field")?;
            build_chain_inner(argument, src, segments)?;
            segments.push(ChainSegment {
                name: node_text(field, src),
                node_kind: field.kind().to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "qualified_identifier" => {
            let scope = node.child_by_field_name("scope");
            let name_node = node.child_by_field_name("name")?;
            if let Some(scope_node) = scope {
                build_chain_inner(scope_node, src, segments)?;
            }
            segments.push(ChainSegment {
                name: node_text(name_node, src),
                node_kind: name_node.kind().to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "call_expression" => {
            let func = node.child_by_field_name("function")?;
            build_chain_inner(func, src, segments)
        }

        _ => None,
    }
}
