// =============================================================================
// java/calls.rs  —  Call extraction and member chain building for Java
// =============================================================================

use super::helpers::{node_text, type_node_simple_name};
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

/// Recursive call extractor that also emits Variable symbols for lambda
/// parameters when a `symbols` vec is provided.
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
            "method_invocation" => {
                // `name` field is always present (identifier).
                if let Some(name_node) = child.child_by_field_name("name") {
                    let chain = build_chain(&child, src);
                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| node_text(name_node, src));
                    if !target_name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: name_node.start_position().row as u32,
                            module: None,
                            chain,
                        });
                    }
                }
                // Recurse into arguments — nested calls.
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }
            "object_creation_expression" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    let name = type_node_simple_name(type_node, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Instantiates,
                            line: type_node.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }
            "lambda_expression" => {
                // Emit Variable symbols for lambda parameters, then recurse into the body.
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_lambda_params(&child, src, source_symbol_index, syms);
                    extract_calls_from_body_with_symbols(&child, src, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, src, source_symbol_index, refs);
                }
            }
            // `if (user instanceof Admin admin)` — Java 16+ pattern matching instanceof.
            // Also handles older `if (user instanceof Admin)` without a pattern variable.
            "instanceof_expression" => {
                extract_instanceof_refs(&child, src, source_symbol_index, refs, symbols.as_deref_mut());
                // Don't recurse further — the instanceof_expression has no nested bodies.
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

/// Emit a TypeRef for `user instanceof Admin` and optionally a Variable symbol
/// for the pattern variable in `user instanceof Admin admin` (Java 16+).
///
/// Tree-sitter-java structure for `instanceof_expression`:
/// ```text
/// instanceof_expression
///   identifier "user"          ← left operand (field: "left")
///   type_identifier "Admin"    ← type (field: "right" or positional)
///   identifier "admin"         ← optional pattern variable
/// ```
fn extract_instanceof_refs(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    symbols: Option<&mut Vec<ExtractedSymbol>>,
) {
    // The type is the "right" field; fall back to scanning for type_identifier.
    let type_node = node.child_by_field_name("right").or_else(|| {
        let mut cursor = node.walk();
        let found = node
            .children(&mut cursor)
            .find(|c| c.kind() == "type_identifier" || c.kind() == "generic_type");
        found
    });

    let type_node = match type_node {
        Some(n) => n,
        None => return,
    };

    let type_name = match type_node.kind() {
        "type_identifier" => node_text(type_node, src),
        "generic_type" => {
            // `instanceof List<String>` — emit base type
            type_node
                .child_by_field_name("name")
                .map(|n| node_text(n, src))
                .unwrap_or_else(|| node_text(type_node, src))
        }
        _ => node_text(type_node, src),
    };

    if type_name.is_empty() {
        return;
    }

    refs.push(ExtractedRef {
        source_symbol_index,
        target_name: type_name.clone(),
        kind: EdgeKind::TypeRef,
        line: type_node.start_position().row as u32,
        module: None,
        chain: None,
    });

    // Pattern variable: the identifier after the type, if present.
    // `user instanceof Admin admin` → emit Variable symbol `admin` with TypeRef to Admin.
    if let Some(syms) = symbols {
        // Find an identifier that comes after the type node.
        let type_end = type_node.end_byte();
        let mut cursor = node.walk();
        for c in node.children(&mut cursor) {
            if c.kind() == "identifier" && c.start_byte() > type_end {
                let var_name = node_text(c, src);
                if !var_name.is_empty() {
                    let var_idx = syms.len();
                    syms.push(make_variable_symbol(var_name, &c, source_symbol_index));
                    refs.push(ExtractedRef {
                        source_symbol_index: var_idx,
                        target_name: type_name.clone(),
                        kind: EdgeKind::TypeRef,
                        line: c.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
                break;
            }
        }
    }
}

/// Extract parameters from a `lambda_expression` node and emit them as
/// Variable symbols.
///
/// Java lambda parameters can be:
/// - A bare `identifier`  → `x -> ...`
/// - `formal_parameters`  → `(String x, int y) -> ...`
/// - `inferred_parameters` → `(x, y) -> ...`
fn extract_lambda_params(
    lambda_node: &Node,
    src: &[u8],
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = lambda_node.walk();
    for child in lambda_node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                // Single untyped parameter: `x -> ...`
                let name = node_text(child, src);
                if !name.is_empty() {
                    symbols.push(make_variable_symbol(name, &child, parent_index));
                }
                // Only the first identifier before `->` is the parameter.
                break;
            }
            "inferred_parameters" => {
                // `(x, y) -> ...` — bare identifier list.
                let mut ic = child.walk();
                for param in child.children(&mut ic) {
                    if param.kind() == "identifier" {
                        let name = node_text(param, src);
                        if !name.is_empty() {
                            symbols.push(make_variable_symbol(name, &param, parent_index));
                        }
                    }
                }
            }
            "formal_parameters" => {
                // `(Type x, Type y) -> ...`
                let mut fp = child.walk();
                for param in child.children(&mut fp) {
                    if param.kind() == "formal_parameter" {
                        if let Some(name_node) = param.child_by_field_name("name") {
                            let name = node_text(name_node, src);
                            if !name.is_empty() {
                                symbols.push(make_variable_symbol(name, &name_node, parent_index));
                            }
                        }
                    }
                }
            }
            "->" => break,
            _ => {}
        }
    }
}

fn make_variable_symbol(name: String, node: &Node, parent_index: usize) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.clone(),
        qualified_name: name,
        kind: SymbolKind::Variable,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: Some(parent_index),
    }
}

// ---------------------------------------------------------------------------
// Member chain builder
// ---------------------------------------------------------------------------

/// Build a structured member access chain from a Java CST node.
///
/// Java uses `method_invocation` for calls and `field_access` for member reads:
///
/// ```text
/// method_invocation
///   object: method_invocation      ← chained call
///     object: identifier "this"
///     name:   identifier "getRepo"
///   name: identifier "findOne"
/// ```
/// produces: `[this, getRepo, findOne]`
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
        "identifier" => {
            let name = node_text(*node, src);
            let kind = SegmentKind::Identifier;
            segments.push(ChainSegment {
                name,
                node_kind: "identifier".to_string(),
                kind,
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

        "super" => {
            segments.push(ChainSegment {
                name: "super".to_string(),
                node_kind: "super".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "method_invocation" => {
            // method_invocation { object?: <expr>, name: identifier }
            if let Some(object) = node.child_by_field_name("object") {
                build_chain_inner(&object, src, segments)?;
            }
            let name_node = node.child_by_field_name("name")?;
            segments.push(ChainSegment {
                name: node_text(name_node, src),
                node_kind: "method_invocation".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "field_access" => {
            // field_access { object: <expr>, field: identifier }
            let object = node.child_by_field_name("object")?;
            let field = node.child_by_field_name("field")?;
            build_chain_inner(&object, src, segments)?;
            segments.push(ChainSegment {
                name: node_text(field, src),
                node_kind: "field_access".to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        _ => None,
    }
}
