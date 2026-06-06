// =============================================================================
// scala/calls.rs  —  Call extraction and member chain builder for Scala
// =============================================================================

use super::decorators::extract_match_patterns;
use super::helpers::{call_target_name, node_text};
use crate::types::{CallArg, ChainSegment, EdgeKind, ExtractedRef, MemberChain, SegmentKind};
use tree_sitter::Node;

#[cfg(test)]
#[path = "calls_tests.rs"]
mod tests;

pub(super) fn extract_call_args(call_node: &Node, src: &[u8]) -> Vec<CallArg> {
    let mut args_node: Option<Node> = None;
    let mut cursor = call_node.walk();
    for c in call_node.children(&mut cursor) {
        match c.kind() {
            "arguments" | "argument_list" => {
                args_node = Some(c);
                break;
            }
            // Brace-block call form `list.map { x => x.foo }`: the `arguments`
            // field is a `block` whose lambda lives as a `lambda_expression`
            // named child. Emit only its lambda param names; a non-lambda block
            // (`{ val t = x; t.foo }`) carries no `lambda_expression` child and
            // contributes nothing, preserving current behavior.
            "block" => {
                let mut bc = c.walk();
                if let Some(lambda) = c
                    .named_children(&mut bc)
                    .find(|n| n.kind() == "lambda_expression")
                {
                    return vec![CallArg::Lambda {
                        params: scala_lambda_param_names(&lambda, src),
                    }];
                }
                return Vec::new();
            }
            _ => {}
        }
    }
    let Some(args) = args_node else { return Vec::new() };
    let mut out = Vec::new();
    let mut ac = args.walk();
    for child in args.named_children(&mut ac) {
        let arg = match child.kind() {
            "string" | "string_literal" | "interpolated_string_expression" => {
                let raw = node_text(child, src);
                CallArg::StringLit(strip_scala_string(&raw))
            }
            "identifier" | "stable_identifier" => CallArg::Ident(node_text(child, src)),
            "integer_literal" | "floating_point_literal" => {
                CallArg::Literal(node_text(child, src))
            }
            "boolean_literal" | "null_literal" => CallArg::Literal(child.kind().to_string()),
            // `x => x.foo`, `(a, b) => f(a, b)` — capture the lambda's own
            // positional parameter names so the chain walker can type them from
            // the higher-order method's callback-parameter signature.
            "lambda_expression" => CallArg::Lambda {
                params: scala_lambda_param_names(&child, src),
            },
            _ => CallArg::Other,
        };
        out.push(arg);
    }
    out
}

/// Collect the positional parameter identifier names of a Scala
/// `lambda_expression` argument. The `parameters` field is either a single
/// `identifier` (`x => ...`) or a `bindings` node of `binding` children whose
/// `name` field is the parameter identifier (`(a, b) => ...`). A binding
/// without a plain `name` identifier yields an empty slot so positions stay
/// aligned with the callback signature.
fn scala_lambda_param_names(node: &Node, src: &[u8]) -> Vec<String> {
    let Some(params) = node.child_by_field_name("parameters") else {
        return Vec::new();
    };
    match params.kind() {
        "identifier" => vec![node_text(params, src)],
        "bindings" => {
            let mut cursor = params.walk();
            params
                .named_children(&mut cursor)
                .filter(|b| b.kind() == "binding")
                .map(|b| {
                    b.child_by_field_name("name")
                        .map(|n| node_text(n, src))
                        .unwrap_or_default()
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn strip_scala_string(raw: &str) -> String {
    let s = raw.trim();
    let s = s.trim_start_matches("\"\"\"").trim_end_matches("\"\"\"");
    let s = s.trim_start_matches('"').trim_end_matches('"');
    let s = if let Some(rest) = s.strip_prefix("s\"") { rest.trim_end_matches('"') } else { s };
    s.to_string()
}

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
                    crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &callee, refs);
                    if !target_name.is_empty() {
                        let call_args = extract_call_args(&child, src);
                        refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: callee.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain,
                            byte_offset: callee.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args,
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
                        refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: op.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain: None,
                            byte_offset: op.start_byte() as u32,
                                                    namespace_segments: Vec::new(),
                                                    call_args: Vec::new(),
});
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // `new Dog(args)` — emit TypeRef to the constructed type.
            "instance_expression" => {
                // Find the type_identifier child (the class name).
                let mut ic = child.walk();
                for inner in child.children(&mut ic) {
                    match inner.kind() {
                        "type_identifier" => {
                            let name = node_text(inner, src);
                            if !name.is_empty() {
                                refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                                    source_symbol_index,
                                    target_name: name,
                                    kind: EdgeKind::Calls,
                                    line: inner.start_position().row as u32,
                                    col: 0,
                                    module: None,
                                    chain: None,
                                    byte_offset: inner.start_byte() as u32,
                                                                    namespace_segments: Vec::new(),
                                                                    call_args: Vec::new(),
});
                            }
                        }
                        // `stable_type_identifier` = qualified type like `foo.Bar`
                        "stable_type_identifier" => {
                            let name = node_text(inner, src);
                            let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
                            if !simple.is_empty() {
                                refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                                    source_symbol_index,
                                    target_name: simple,
                                    kind: EdgeKind::Calls,
                                    line: inner.start_position().row as u32,
                                    col: 0,
                                    module: Some(name),
                                    chain: None,
                                    byte_offset: inner.start_byte() as u32,
                                                                    namespace_segments: Vec::new(),
                                                                    call_args: Vec::new(),
});
                            }
                        }
                        _ => {}
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // Lambda expressions: `x => expr`, `(x, y) => expr` — recurse into body.
            "lambda_expression" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            // type_arguments may appear in expressions (e.g. generic method calls).
            "type_arguments" => {
                extract_type_refs_from_type_arguments(&child, src, source_symbol_index, refs);
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }

            _ => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Extract TypeRef edges from type_arguments node (e.g., `List[User]` → TypeRef to User).
/// NOTE: We extract ALL type identifiers, including builtins. Filtering happens in resolution.
fn extract_type_refs_from_type_arguments(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" => {
                let name = node_text(child, src);
                if !name.is_empty() {
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }
            "generic_type" | "compound_type" | "function_type" | "type_arguments" => {
                // Recurse into nested types.
                extract_type_refs_from_type_arguments(&child, src, source_symbol_index, refs);
            }
            _ => {
                // Keep recursing to find type_identifier nodes.
                extract_type_refs_from_type_arguments(&child, src, source_symbol_index, refs);
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
                byte_offset: 0,
                            declared_type_id: None,
                is_call: false,
                call_args: Vec::new(),
                type_arg_ids: Vec::new(),
});
            Some(())
        }

        "call_expression" => {
            // Chained call: function child carries the chain. Mark the invoked
            // segment so the walker yields the function's return type rather than
            // the function value (closure call-yield; Scala uses `T => R`).
            let callee = node
                .child_by_field_name("function")
                .or_else(|| node.named_child(0))?;
            build_chain_inner(&callee, src, segments)?;
            if let Some(last) = segments.last_mut() {
                last.is_call = true;
            }
            Some(())
        }

        _ => None,
    }
}
