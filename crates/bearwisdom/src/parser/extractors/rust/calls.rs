// =============================================================================
// rust/calls.rs  —  Call and use-declaration extraction for Rust
// =============================================================================

use super::helpers::node_text;
use super::symbols::extract_method_from_fn;
use crate::types::{ChainSegment, EdgeKind, ExtractedRef, ExtractedSymbol, MemberChain, SegmentKind, SymbolKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// impl block handling
// ---------------------------------------------------------------------------

/// Process an `impl_item` — not a symbol itself, but the container for methods.
/// The implementing type name becomes the qualified prefix for its methods.
pub(super) fn extract_impl(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    outer_prefix: &str,
) {
    let type_node = match node.child_by_field_name("type") {
        Some(n) => n,
        None => return,
    };
    let type_name = node_text(&type_node, source);

    let impl_prefix = if outer_prefix.is_empty() {
        type_name
    } else {
        format!("{outer_prefix}.{type_name}")
    };

    let body = match node.child_by_field_name("body") {
        Some(b) => b,
        None => return,
    };

    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "function_item" {
            if let Some(sym) = extract_method_from_fn(&child, source, None, &impl_prefix) {
                let idx = symbols.len();
                symbols.push(sym);
                if let Some(fn_body) = child.child_by_field_name("body") {
                    extract_calls_from_body_with_symbols(&fn_body, source, idx, refs, Some(symbols));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

/// Recursively scan a function/method body for `call_expression` nodes
/// and emit `Calls` references.
pub(super) fn extract_calls_from_body(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    extract_calls_from_body_with_symbols(node, source, source_symbol_index, refs, None);
}

/// Variant that also emits Variable symbols for closure parameters.
pub(super) fn extract_calls_from_body_with_symbols(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    mut symbols: Option<&mut Vec<ExtractedSymbol>>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "call_expression" => {
                if let Some(func) = child.child_by_field_name("function") {
                    let chain = build_chain(func, source);

                    let target_name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| {
                            let callee_text = node_text(&func, source);
                            callee_text
                                .rsplit("::")
                                .next()
                                .unwrap_or(&callee_text)
                                .rsplit('.')
                                .next()
                                .unwrap_or(&callee_text)
                                .trim()
                                .to_string()
                        });

                    if !target_name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name,
                            kind: EdgeKind::Calls,
                            line: func.start_position().row as u32,
                            module: None,
                            chain,
                        });
                    }
                }
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, source, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, source, source_symbol_index, refs);
                }
            }

            "closure_expression" => {
                // Emit Variable symbols for closure parameters, then recurse into body.
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_closure_params(&child, source, source_symbol_index, syms);
                    extract_calls_from_body_with_symbols(&child, source, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, source, source_symbol_index, refs);
                }
            }

            _ => {
                if let Some(syms) = symbols.as_deref_mut() {
                    extract_calls_from_body_with_symbols(&child, source, source_symbol_index, refs, Some(syms));
                } else {
                    extract_calls_from_body(&child, source, source_symbol_index, refs);
                }
            }
        }
    }
}

/// Emit Variable symbols for each identifier in a `closure_parameters` node.
///
/// Handles:
/// - `|x|`             → identifier
/// - `|x: Type|`       → identifier with type annotation
/// - `|mut x|`         → mutable binding
/// - `|(a, b)|`        → destructured tuple pattern
fn extract_closure_params(
    closure_node: &Node,
    source: &str,
    parent_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = closure_node.walk();
    for child in closure_node.children(&mut cursor) {
        if child.kind() == "closure_parameters" {
            let mut pc = child.walk();
            for param in child.children(&mut pc) {
                match param.kind() {
                    "identifier" => {
                        let name = node_text(&param, source);
                        if !name.is_empty() && name != "|" {
                            symbols.push(make_closure_variable(name, &param, parent_index));
                        }
                    }
                    // `x: Type` — the identifier is a child named `pattern`
                    "parameter" => {
                        if let Some(pat) = param.child_by_field_name("pattern") {
                            let name = node_text(&pat, source);
                            if !name.is_empty() {
                                symbols.push(make_closure_variable(name, &pat, parent_index));
                            }
                        }
                    }
                    // `mut x`
                    "mut_specifier" | "mutable_specifier" => {}
                    _ => {}
                }
            }
        }
    }
}

fn make_closure_variable(name: String, node: &Node, parent_index: usize) -> ExtractedSymbol {
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

/// Build a structured member-access chain from a Rust call expression's function node.
///
/// Returns `None` for bare single-segment identifiers.
fn build_chain(node: Node, source: &str) -> Option<MemberChain> {
    if node.kind() == "identifier" || node.kind() == "self" {
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
            });
            Some(())
        }

        "scoped_identifier" => {
            let text = node_text(&node, source);
            let parts: Vec<&str> = text.split("::").collect();
            if parts.len() < 2 {
                segments.push(ChainSegment {
                    name: text,
                    node_kind: "scoped_identifier".to_string(),
                    kind: SegmentKind::Identifier,
                    declared_type: None,
                    type_args: vec![],
                    optional_chaining: false,
                });
            } else {
                for (i, part) in parts.iter().enumerate() {
                    let kind = if i == 0 {
                        SegmentKind::Identifier
                    } else {
                        SegmentKind::Property
                    };
                    segments.push(ChainSegment {
                        name: part.trim().to_string(),
                        node_kind: "scoped_identifier".to_string(),
                        kind,
                        declared_type: None,
                        type_args: vec![],
                        optional_chaining: false,
                    });
                }
            }
            Some(())
        }

        "call_expression" => {
            let func = node.child_by_field_name("function")?;
            build_chain_inner(func, source, segments)
        }

        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Use declaration / import reference extraction
// ---------------------------------------------------------------------------

/// Walk a `use_declaration` node and emit `Import` references for every
/// leaf name that is actually imported.
pub(super) fn extract_use_names(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "scoped_identifier"
            | "scoped_use_list"
            | "use_as_clause"
            | "use_wildcard"
            | "identifier"
            | "use_list" => {
                walk_use_tree(&child, source, refs, current_symbol_count, "");
            }
            _ => {}
        }
    }
}

fn walk_use_tree(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    current_symbol_count: usize,
    prefix: &str,
) {
    match node.kind() {
        "scoped_identifier" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| node_text(&n, source))
                .unwrap_or_default();
            let path = node
                .child_by_field_name("path")
                .map(|n| node_text(&n, source))
                .unwrap_or_default();

            if name.is_empty() {
                return;
            }

            let module = build_module_path(prefix, &path);
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: name,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module: if module.is_empty() { None } else { Some(module) },
                chain: None,
            });
        }

        "scoped_use_list" => {
            let path = node
                .child_by_field_name("path")
                .map(|n| node_text(&n, source))
                .unwrap_or_default();
            let new_prefix = build_module_path(prefix, &path);

            if let Some(list) = node.child_by_field_name("list") {
                walk_use_tree(&list, source, refs, current_symbol_count, &new_prefix);
            }
        }

        "use_list" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "{" | "}" | "," => {}
                    _ => walk_use_tree(&child, source, refs, current_symbol_count, prefix),
                }
            }
        }

        "use_as_clause" => {
            let alias = node
                .child_by_field_name("alias")
                .map(|n| node_text(&n, source));
            let original = node
                .child_by_field_name("path")
                .map(|n| node_text(&n, source));

            let target = alias.or(original).unwrap_or_default();
            if target.is_empty() {
                return;
            }

            let module = if prefix.is_empty() {
                None
            } else {
                Some(prefix.to_string())
            };

            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: target,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module,
                chain: None,
            });
        }

        "use_wildcard" => {
            let module = if prefix.is_empty() {
                None
            } else {
                Some(prefix.to_string())
            };
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: "*".to_string(),
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module,
                chain: None,
            });
        }

        "identifier" => {
            let name = node_text(node, source);
            if name.is_empty() {
                return;
            }
            let module = if prefix.is_empty() {
                None
            } else {
                Some(prefix.to_string())
            };
            refs.push(ExtractedRef {
                source_symbol_index: current_symbol_count,
                target_name: name,
                kind: EdgeKind::Imports,
                line: node.start_position().row as u32,
                module,
                chain: None,
            });
        }

        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk_use_tree(&child, source, refs, current_symbol_count, prefix);
            }
        }
    }
}

fn build_module_path(prefix: &str, path: &str) -> String {
    match (prefix.is_empty(), path.is_empty()) {
        (true, true) => String::new(),
        (true, false) => path.to_string(),
        (false, true) => prefix.to_string(),
        (false, false) => format!("{prefix}::{path}"),
    }
}

