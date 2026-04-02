// =============================================================================
// parser/extractors/scala/mod.rs  —  Scala symbol and reference extractor
// =============================================================================


use super::{calls, symbols, helpers, decorators};
use super::calls::extract_calls_from_body;
use super::decorators::{extract_case_class_params, extract_decorators, extract_match_patterns};
use super::helpers::{call_target_name, classify_class, node_text};
use super::symbols::{
    extract_enum_body, extract_extends_with, push_extension_definition, push_function_def,
    push_given_definition, push_import, push_type_def, push_type_definition, push_val_var,
    recurse_body,
};

use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

pub(crate) static SCALA_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "class_definition",    name_field: "name" },
    ScopeKind { node_kind: "object_definition",   name_field: "name" },
    ScopeKind { node_kind: "trait_definition",    name_field: "name" },
    ScopeKind { node_kind: "enum_definition",     name_field: "name" },
    ScopeKind { node_kind: "function_definition", name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn extract(source: &str) -> super::ExtractionResult {
    let lang: tree_sitter::Language = tree_sitter_scala::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&lang)
        .expect("Failed to load Scala grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let root = tree.root_node();
    let src = source.as_bytes();
    let has_errors = root.has_error();

    let scope_tree = scope_tree::build(root, src, SCALA_SCOPE_KINDS);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_node(root, src, &scope_tree, &mut symbols, &mut refs, None);

    super::ExtractionResult::new(symbols, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Recursive node visitor
// ---------------------------------------------------------------------------

pub(super) fn extract_node<'a>(
    node: Node<'a>,
    src: &[u8],
    scope_tree: &scope_tree::ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_declaration" => {
                push_import(&child, src, symbols.len(), refs);
            }

            "class_definition" => {
                let kind = classify_class(&child, src);
                let idx = push_type_def(&child, src, scope_tree, kind, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_extends_with(&child, src, sym_idx, refs);
                    // Extract case class constructor params as Property symbols.
                    let qname = symbols[sym_idx].qualified_name.clone();
                    extract_case_class_params(&child, src, sym_idx, &qname, symbols);
                }
                recurse_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "object_definition" => {
                let idx =
                    push_type_def(&child, src, scope_tree, SymbolKind::Namespace, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_extends_with(&child, src, sym_idx, refs);
                }
                recurse_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "trait_definition" => {
                let idx =
                    push_type_def(&child, src, scope_tree, SymbolKind::Interface, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_extends_with(&child, src, sym_idx, refs);
                }
                recurse_body(&child, src, scope_tree, symbols, refs, idx);
            }

            // Scala 3 enum
            "enum_definition" => {
                let idx =
                    push_type_def(&child, src, scope_tree, SymbolKind::Enum, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    extract_extends_with(&child, src, sym_idx, refs);
                }
                extract_enum_body(&child, src, scope_tree, symbols, refs, idx);
            }

            "function_definition" => {
                let idx = push_function_def(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    if let Some(body) = child.child_by_field_name("body") {
                        // If the body IS a match_expression (e.g. `def f = x match {...}`),
                        // extract patterns directly; extract_calls_from_body only sees children.
                        if body.kind() == "match_expression" {
                            extract_match_patterns(&body, src, sym_idx, refs);
                        }
                        // For expression-body functions (`def f = expr`), the body may be an
                        // infix_expression or call_expression directly — handle the root too.
                        dispatch_body_node(body, src, sym_idx, refs);
                        extract_calls_from_body(&body, src, sym_idx, refs);
                    }
                }
            }

            "val_definition" | "var_definition" => {
                push_val_var(&child, src, scope_tree, symbols, parent_index);
            }

            // Scala `type` alias / abstract type member.
            "type_definition" | "type_declaration" => {
                push_type_definition(&child, src, scope_tree, symbols, refs, parent_index);
            }

            // Scala 3 `given` — implicit instance.
            "given_definition" => {
                let idx = push_given_definition(&child, src, scope_tree, symbols, refs, parent_index);
                recurse_body(&child, src, scope_tree, symbols, refs, idx);
            }

            // Scala 3 `extension` — extension methods block.
            "extension_definition" => {
                let idx = push_extension_definition(&child, src, scope_tree, symbols, parent_index);
                recurse_body(&child, src, scope_tree, symbols, refs, idx);
            }

            // `package foo.bar { ... }` — recurse into the body.
            "package_clause" => {
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, parent_index);
                } else {
                    extract_node(child, src, scope_tree, symbols, refs, parent_index);
                }
            }

            "match_expression" => {
                if let Some(sym_idx) = parent_index {
                    extract_match_patterns(&child, src, sym_idx, refs);
                }
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            // for-expression / for-comprehension — extract embedded calls.
            "for_expression" => {
                if let Some(sym_idx) = parent_index {
                    extract_calls_from_body(&child, src, sym_idx, refs);
                }
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Dispatch a single expression node that may be the direct body of a function
/// (i.e. not a block container). Handles infix_expression and call_expression
/// that would otherwise be missed because `extract_calls_from_body` only walks
/// children of the passed node.
fn dispatch_body_node(
    node: tree_sitter::Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "infix_expression" => {
            if let Some(op) = node.child_by_field_name("operator") {
                let target_name = node_text(op, src);
                if !target_name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name,
                        kind: crate::types::EdgeKind::Calls,
                        line: op.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
        }
        "call_expression" => {
            if let Some(callee) = node
                .child_by_field_name("function")
                .or_else(|| node.named_child(0))
            {
                let chain = calls::build_chain(&callee, src);
                let target_name = chain
                    .as_ref()
                    .and_then(|c| c.segments.last())
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| call_target_name(&callee, src));
                if !target_name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name,
                        kind: crate::types::EdgeKind::Calls,
                        line: callee.start_position().row as u32,
                        module: None,
                        chain,
                    });
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

