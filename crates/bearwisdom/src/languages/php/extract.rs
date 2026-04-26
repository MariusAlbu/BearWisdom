// =============================================================================
// parser/extractors/php/mod.rs  —  PHP symbol and reference extractor
// =============================================================================


use super::{calls, symbols, decorators};
use crate::parser::scope_tree::{self, ScopeKind};
use crate::types::{ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Scope configuration
// ---------------------------------------------------------------------------

pub(crate) static PHP_SCOPE_KINDS: &[ScopeKind] = &[
    ScopeKind { node_kind: "namespace_definition", name_field: "name" },
    ScopeKind { node_kind: "class_declaration",    name_field: "name" },
    ScopeKind { node_kind: "interface_declaration", name_field: "name" },
    ScopeKind { node_kind: "trait_declaration",    name_field: "name" },
    ScopeKind { node_kind: "enum_declaration",     name_field: "name" },
    ScopeKind { node_kind: "method_declaration",   name_field: "name" },
    ScopeKind { node_kind: "function_definition",  name_field: "name" },
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Extract all symbols and references from PHP source code.
pub fn extract(source: &str) -> super::ExtractionResult {
    let language: tree_sitter::Language = tree_sitter_php::LANGUAGE_PHP.into();
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to load PHP grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return super::ExtractionResult::new(vec![], vec![], true),
    };

    let has_errors = tree.root_node().has_error();
    let src = source.as_bytes();
    let root = tree.root_node();

    let _scope_tree = scope_tree::build(root, src, PHP_SCOPE_KINDS);

    let mut syms: Vec<ExtractedSymbol> = Vec::new();
    let mut refs: Vec<ExtractedRef> = Vec::new();

    extract_from_node(root, src, &mut syms, &mut refs, None, "", "");

    // Post-traversal: scan the entire CST for named_type and qualified_name
    // nodes in type positions, emitting TypeRef for any missed by the walker.
    scan_all_type_refs(root, src, &mut refs);

    super::ExtractionResult::new(syms, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

pub(super) fn extract_from_node(
    node: Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    namespace_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "namespace_definition" => {
                symbols::extract_namespace(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            "namespace_use_declaration" => {
                calls::extract_use_declaration(&child, src, refs, symbols.len());
            }

            "function_definition" => {
                let fn_idx = symbols.len();
                symbols::extract_function(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                    false,
                );
                if symbols.len() > fn_idx {
                    decorators::extract_decorators(&child, src, fn_idx, refs);
                }
            }

            "class_declaration" => {
                let class_idx = symbols.len();
                symbols::extract_class(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                    SymbolKind::Class,
                );
                if symbols.len() > class_idx {
                    decorators::extract_decorators(&child, src, class_idx, refs);
                }
            }

            "interface_declaration" => {
                let class_idx = symbols.len();
                symbols::extract_class(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                    SymbolKind::Interface,
                );
                if symbols.len() > class_idx {
                    decorators::extract_decorators(&child, src, class_idx, refs);
                }
            }

            "trait_declaration" => {
                let class_idx = symbols.len();
                symbols::extract_class(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                    SymbolKind::Class,
                );
                if symbols.len() > class_idx {
                    decorators::extract_decorators(&child, src, class_idx, refs);
                }
            }

            "enum_declaration" => {
                let enum_idx = symbols.len();
                symbols::extract_enum(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                );
                if symbols.len() > enum_idx {
                    decorators::extract_decorators(&child, src, enum_idx, refs);
                }
            }

            // `foreach ($items as $key => $value) { ... }`
            "foreach_statement" => {
                let source_idx = parent_index.unwrap_or(0);
                calls::extract_foreach_vars(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    source_idx,
                );
            }

            // `try { ... } catch (Ex $e) { ... } finally { ... }`
            "try_statement" => {
                let source_idx = parent_index.unwrap_or(0);
                calls::extract_try_catch_types(
                    &child,
                    src,
                    refs,
                    symbols,
                    parent_index,
                    qualified_prefix,
                    source_idx,
                );
            }

            // `global $var;` — scope modifier, extract as variable.
            "global_declaration" => {
                symbols::extract_global_static_vars(&child, src, symbols, parent_index, qualified_prefix, false);
            }

            // `static $cache = [];` — static local variable.
            "static_variable_declaration" => {
                symbols::extract_global_static_vars(&child, src, symbols, parent_index, qualified_prefix, true);
            }

            // `[$name] = $user->toArray()` / `list($a, $b) = $tuple`
            "expression_statement" => {
                symbols::extract_expression_statement(
                    &child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
            }

            // Direct call expressions that appear outside an expression_statement
            // (e.g. as the direct body of a namespace block, or in some PHP
            // grammar variants).  Delegate to the body extractor so all call
            // kinds (function call, method call, new) are captured uniformly.
            "function_call_expression"
            | "member_call_expression"
            | "nullsafe_member_call_expression"
            | "static_call_expression"
            | "object_creation_expression" => {
                calls::extract_calls_from_body(
                    &child,
                    src,
                    parent_index.unwrap_or(0),
                    refs,
                );
            }

            // `use TraitName;` inside a class body at top-level traversal
            // (when encountered outside of a class's declaration_list walk).
            "use_declaration" => {
                calls::extract_trait_use(&child, src, refs, symbols.len());
            }

            "ERROR" | "MISSING" => {}

            _ => {
                extract_from_node(
                    child,
                    src,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    namespace_prefix,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Post-traversal full-tree type reference scan
// ---------------------------------------------------------------------------

/// Walk the entire CST and emit TypeRef edges for every `named_type` and
/// `qualified_name` node found in type positions. This catches type references
/// in parameter type declarations, return types, property types, and catch
/// clauses that the top-down walker may have missed.
///
/// PHP primitives (int, float, string, bool, void, null, array, object,
/// mixed, never, callable, iterable, self, static, parent, true, false) are
/// filtered out — they are always available and never in the project index.
fn scan_all_type_refs(node: tree_sitter::Node, src: &[u8], refs: &mut Vec<crate::types::ExtractedRef>) {
    scan_type_refs_inner(node, src, 0, refs);
}

fn scan_type_refs_inner(
    node: tree_sitter::Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<crate::types::ExtractedRef>,
) {
    match node.kind() {
        "named_type" => {
            // named_type is a leaf (or contains a name/qualified_name child).
            // Try the direct text first; if it contains a child, use that.
            let raw = super::helpers::node_text(&node, src);
            // Extract the simple name (last segment after `\`).
            let name = raw
                .trim_start_matches('\\')
                .rsplit('\\')
                .next()
                .unwrap_or(&raw)
                .to_string();
            if !name.is_empty() && !is_php_primitive(&name) {
                refs.push(crate::types::ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: crate::types::EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
            }
        }
        "qualified_name" => {
            let raw = super::helpers::node_text(&node, src);
            let name = raw
                .trim_start_matches('\\')
                .rsplit('\\')
                .next()
                .unwrap_or(&raw)
                .to_string();
            if !name.is_empty() && !is_php_primitive(&name) {
                refs.push(crate::types::ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: crate::types::EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                    byte_offset: 0,
                                    namespace_segments: Vec::new(),
});
            }
        }
        // `instanceof` binary expression — the RHS is a `class_name` node
        // (not `named_type`) in the PHP grammar. Emit TypeRef for the class.
        "binary_expression" => {
            // Check if any direct child is the `instanceof` keyword.
            let mut has_instanceof = false;
            let mut rhs_node: Option<tree_sitter::Node> = None;
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            for (i, child) in children.iter().enumerate() {
                if child.kind() == "instanceof" || super::helpers::node_text(child, src) == "instanceof" {
                    has_instanceof = true;
                    // RHS is the next sibling
                    if let Some(rhs) = children.get(i + 1) {
                        rhs_node = Some(*rhs);
                    }
                    break;
                }
            }
            if has_instanceof {
                if let Some(rhs) = rhs_node {
                    // RHS may be class_name, named_type, qualified_name, or identifier
                    let raw = super::helpers::node_text(&rhs, src);
                    let name = raw
                        .trim_start_matches('\\')
                        .rsplit('\\')
                        .next()
                        .unwrap_or(&raw)
                        .to_string();
                    if !name.is_empty() && !is_php_primitive(&name) {
                        refs.push(crate::types::ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: crate::types::EdgeKind::TypeRef,
                            line: rhs.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
});
                    }
                }
                // Also recurse into the LHS (before instanceof) in case it has nested exprs
                if let Some(lhs) = children.first() {
                    scan_type_refs_inner(*lhs, src, source_symbol_index, refs);
                }
            } else {
                // Not an instanceof — recurse normally
                let mut cursor2 = node.walk();
                for child in node.children(&mut cursor2) {
                    scan_type_refs_inner(child, src, source_symbol_index, refs);
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                scan_type_refs_inner(child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Return true for PHP built-in types that are never in the project index.
fn is_php_primitive(name: &str) -> bool {
    matches!(
        name,
        "int" | "float" | "string" | "bool" | "void" | "null"
            | "array" | "object" | "mixed" | "never" | "callable"
            | "iterable" | "self" | "static" | "parent" | "true" | "false"
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

