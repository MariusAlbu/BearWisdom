// =============================================================================
// ruby/calls.rs  —  Call extraction and member chain builder for Ruby
// =============================================================================

use super::helpers::{get_call_method_name, node_text};
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, SegmentKind, SymbolKind};
use tree_sitter::Node;

pub(super) fn extract_calls_from_body(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    extract_calls_from_body_with_symbols(node, src, source_symbol_index, refs, None);
}

/// Recursive call extractor that also emits Variable symbols for block parameters
/// when a `symbols` vec is provided.
pub(super) fn extract_calls_from_body_with_symbols(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    mut symbols: Option<&mut Vec<ExtractedSymbol>>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call" => {
                if let Some(mname) = get_call_method_name(&child, src) {
                    if mname == "new" {
                        // Emit Instantiates for `ClassName.new`
                        if let Some(recv) = child.child_by_field_name("receiver") {
                            let recv_text = node_text(&recv, src);
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: recv_text,
                                kind: EdgeKind::Instantiates,
                                line: child.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                            // Don't also emit a Calls edge for `.new`.
                            // Recurse into arguments but not the receiver again.
                            if let Some(syms) = symbols.as_deref_mut() {
                                extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                            } else {
                                extract_calls_from_body(&child, src, source_symbol_index, refs);
                            }
                            continue;
                        }
                    }

                    let chain = build_chain(&child, src);
                    super::crate::parser::extractors::emit_chain_type_ref(&chain, source_symbol_index, &child, refs);
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: mname,
                        kind: EdgeKind::Calls,
                        line: child.start_position().row as u32,
                        module: None,
                        chain,
                    });
                }
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }

            // `puts "hello"` / `raise NotImplementedError` — method call without parens.
            "command_call" => {
                // receiver (optional) + method identifier.
                if let Some(method_node) = child.child_by_field_name("method") {
                    let mname = node_text(&method_node, src);
                    if !mname.is_empty() {
                        let chain = build_chain(&child, src);
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: mname,
                            kind: EdgeKind::Calls,
                            line: method_node.start_position().row as u32,
                            module: None,
                            chain,
                        });
                    }
                } else {
                    // Bare command: first identifier is the method name.
                    let mut cc = child.walk();
                    for gc in child.children(&mut cc) {
                        if gc.kind() == "identifier" || gc.kind() == "constant" {
                            let mname = node_text(&gc, src);
                            if !mname.is_empty() {
                                refs.push(ExtractedRef {
                                    source_symbol_index,
                                    target_name: mname,
                                    kind: EdgeKind::Calls,
                                    line: gc.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                });
                            }
                            break;
                        }
                    }
                }
                // Recurse into arguments for nested calls.
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }

            // `obj.method arg` — method call node (grammar variant of `call`).
            "method_call" => {
                if let Some(method_node) = child.child_by_field_name("method") {
                    let mname = node_text(&method_node, src);
                    if !mname.is_empty() {
                        let chain = build_chain(&child, src);
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: mname,
                            kind: EdgeKind::Calls,
                            line: method_node.start_position().row as u32,
                            module: None,
                            chain,
                        });
                    }
                }
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }

            "block" | "do_block" => {
                // Extract block parameters as Variable symbols, then recurse into
                // the block body so calls inside blocks are captured.
                let params_kind = "block_parameters";
                // Emit Variable symbols for block parameters.
                let mut cc = child.walk();
                for block_child in child.children(&mut cc) {
                    if block_child.kind() == params_kind {
                        if let Some(syms) = symbols.as_deref_mut() {
                            extract_block_params(&block_child, src, source_symbol_index, syms);
                        }
                    }
                }
                // Recurse into block body.
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }

            // `"Hello #{user.get_name()}"` — extract calls from string interpolations.
            "string" | "subshell" => {
                extract_string_interpolation_calls(&child, src, source_symbol_index, refs, symbols.as_deref_mut());
            }

            // `case x; when A then ...; when B then ...; end`
            "case" => {
                extract_case_calls(&child, src, source_symbol_index, refs, symbols.as_deref_mut());
            }

            // `begin; ...; end` blocks — recurse into body.
            "begin_block" | "begin" => {
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }

            // `ensure` clause — recurse.
            "ensure" => {
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }

            // `hash` / `array` — recurse for calls in values/elements.
            "hash" | "array" => {
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }

            _ => {
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }
        }
    }
}

/// Emit a `Variable` symbol for each identifier in a `block_parameters` node.
fn extract_block_params(
    params_node: &Node,
    src: &[u8],
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        if child.kind() == "identifier" {
            let name = node_text(&child, src);
            if name.is_empty() {
                continue;
            }
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: name,
                kind: SymbolKind::Variable,
                visibility: None,
                start_line: child.start_position().row as u32,
                end_line: child.end_position().row as u32,
                start_col: child.start_position().column as u32,
                end_col: child.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: None,
                parent_index: Some(parent_index),
            });
        }
    }
}

/// Extract calls from string interpolation expressions: `"Hello #{user.name}"`.
///
/// tree-sitter-ruby represents interpolation as:
/// ```text
/// string
///   string_content
///   interpolation        ← #{...}
///     <expression>
///   string_content
/// ```
fn extract_string_interpolation_calls(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    mut symbols: Option<&mut Vec<ExtractedSymbol>>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "interpolation" {
            // Recurse into the interpolation node itself so that call nodes
            // inside it are discovered by the `"call"` match arm in
            // `extract_calls_from_body_with_symbols`.  Passing the individual
            // named children would mean the function sees the *inside* of the
            // call (identifiers, argument_list) rather than the call itself.
            if let Some(syms) = symbols.as_deref_mut() {
                extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
            } else {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Extract calls from all `when` arms and the `else` arm of a `case` statement.
fn extract_case_calls(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    mut symbols: Option<&mut Vec<ExtractedSymbol>>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Recurse into the subject expression.
            "value" => {
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }
            // `when <pattern> then <body>` — recurse into body for calls.
            "when" => {
                // The body of a `when` clause is everything after the pattern.
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }
            // `else` branch.
            "else" => {
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }
            _ => {
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
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
        "self" => {
            segments.push(ChainSegment {
                name: "self".to_string(),
                node_kind: "self".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "identifier" | "constant" => {
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

        "call" => {
            // `receiver.method(...)` — recurse into receiver, then push method.
            if let Some(receiver) = node.child_by_field_name("receiver") {
                build_chain_inner(&receiver, src, segments)?;
                if let Some(method) = node.child_by_field_name("method") {
                    segments.push(ChainSegment {
                        name: node_text(&method, src),
                        node_kind: "call".to_string(),
                        kind: SegmentKind::Property,
                        declared_type: None,
                        type_args: vec![],
                        optional_chaining: false,
                    });
                }
                Some(())
            } else {
                // Bare call (no receiver) — treat the method name as Identifier.
                if let Some(method) = node.child_by_field_name("method") {
                    segments.push(ChainSegment {
                        name: node_text(&method, src),
                        node_kind: "call".to_string(),
                        kind: SegmentKind::Identifier,
                        declared_type: None,
                        type_args: vec![],
                        optional_chaining: false,
                    });
                    Some(())
                } else {
                    None
                }
            }
        }

        _ => None,
    }
}
