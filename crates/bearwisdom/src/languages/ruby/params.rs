// =============================================================================
// ruby/params.rs  —  Method parameter and rescue-clause extraction for Ruby
//
// What we extract
// ---------------
// From method parameters:
//   keyword_parameter  (name:)        → Variable symbol
//   optional_parameter (name = val)   → Variable symbol
//   block_parameter    (&block)        → Variable symbol
//   splat_parameter    (*args)         → Variable symbol
//   hash_splat_parameter (**opts)      → Variable symbol
//   identifier (plain positional)     → Variable symbol
//
// From rescue clauses:
//   exception type constants           → TypeRef edges
//   rescue variable (`=> e`)           → Variable symbol
// =============================================================================

use super::helpers::{node_text, scope_from_prefix};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Method parameters
// ---------------------------------------------------------------------------

/// Extract all parameter names from a `method_parameters` (or `lambda_parameters`)
/// node and emit them as Variable symbols scoped to `parent_index`.
pub(super) fn extract_method_params(
    params_node: &Node,
    src: &[u8],
    parent_index: usize,
    qualified_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        match child.kind() {
            // Plain positional: `def foo(name)`
            "identifier" => {
                let name = node_text(&child, src);
                if !name.is_empty() {
                    symbols.push(make_param_variable(name, &child, parent_index, qualified_prefix));
                }
            }

            // Keyword: `name:` or `name: default`
            "keyword_parameter" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(&name_node, src);
                    if !name.is_empty() {
                        symbols.push(make_param_variable(name, &name_node, parent_index, qualified_prefix));
                    }
                }
            }

            // Optional: `name = default`
            "optional_parameter" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(&name_node, src);
                    if !name.is_empty() {
                        symbols.push(make_param_variable(name, &name_node, parent_index, qualified_prefix));
                    }
                }
            }

            // Block: `&block`
            "block_parameter" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(&name_node, src);
                    if !name.is_empty() {
                        symbols.push(make_param_variable(name, &name_node, parent_index, qualified_prefix));
                    }
                } else {
                    // Fallback: grab the first identifier child after `&`
                    let mut cc = child.walk();
                    for c in child.children(&mut cc) {
                        if c.kind() == "identifier" {
                            let name = node_text(&c, src);
                            if !name.is_empty() {
                                symbols.push(make_param_variable(name, &c, parent_index, qualified_prefix));
                                break;
                            }
                        }
                    }
                }
            }

            // Splat: `*args`
            "splat_parameter" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(&name_node, src);
                    if !name.is_empty() {
                        symbols.push(make_param_variable(name, &name_node, parent_index, qualified_prefix));
                    }
                }
            }

            // Hash splat: `**opts`
            "hash_splat_parameter" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(&name_node, src);
                    if !name.is_empty() {
                        symbols.push(make_param_variable(name, &name_node, parent_index, qualified_prefix));
                    }
                }
            }

            // Destructured array param: `(a, b)`
            "destructured_parameter" => {
                extract_destructured_param(&child, src, parent_index, qualified_prefix, symbols);
            }

            _ => {}
        }
    }
}

fn extract_destructured_param(
    node: &Node,
    src: &[u8],
    parent_index: usize,
    qualified_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            let name = node_text(&child, src);
            if !name.is_empty() {
                symbols.push(make_param_variable(name, &child, parent_index, qualified_prefix));
            }
        } else if child.kind() == "destructured_parameter" {
            extract_destructured_param(&child, src, parent_index, qualified_prefix, symbols);
        }
    }
}

// ---------------------------------------------------------------------------
// Rescue clauses
// ---------------------------------------------------------------------------

/// Extract TypeRef edges for exception types and a Variable symbol for the
/// rescue variable (`rescue SomeError => e`).
///
/// Handles:
///   `rescue ActiveRecord::RecordNotFound => e`
///   `rescue StandardError, RuntimeError => e`
///   `rescue => e`   (bare rescue, no explicit type)
pub(super) fn extract_rescue(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `exceptions` field holds the list of exception types
            "exceptions" => {
                let mut ec = child.walk();
                for exc in child.children(&mut ec) {
                    match exc.kind() {
                        "constant" | "scope_resolution" => {
                            let type_name = node_text(&exc, src);
                            if !type_name.is_empty() {
                                refs.push(ExtractedRef {
                                    source_symbol_index,
                                    target_name: type_name,
                                    kind: EdgeKind::TypeRef,
                                    line: exc.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }

            // `variable` field is the `=> e` part
            "exception_variable" => {
                // The `exception_variable` node wraps `=> identifier`
                let mut vc = child.walk();
                for v_child in child.children(&mut vc) {
                    if v_child.kind() == "identifier" {
                        let name = node_text(&v_child, src);
                        if !name.is_empty() {
                            symbols.push(make_param_variable(name, &v_child, source_symbol_index, qualified_prefix));
                        }
                    }
                }
            }

            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn make_param_variable(
    name: String,
    node: &Node,
    parent_index: usize,
    qualified_prefix: &str,
) -> ExtractedSymbol {
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
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index: Some(parent_index),
    }
}
