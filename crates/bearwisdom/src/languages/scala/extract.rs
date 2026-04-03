// =============================================================================
// parser/extractors/scala/mod.rs  —  Scala symbol and reference extractor
// =============================================================================


use super::{calls, symbols, helpers, decorators};
use super::calls::extract_calls_from_body;
use super::decorators::{extract_case_class_params, extract_decorators, extract_match_patterns};
use super::helpers::{call_target_name, classify_class, node_text};
use super::symbols::{
    extract_enum_body, extract_extends_with, push_export, push_extension_definition,
    push_function_def, push_given_definition, push_import, push_package_clause, push_type_def,
    push_type_definition, push_val_var, recurse_body,
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

    // Post-traversal: scan the entire CST for type_identifier nodes and emit
    // TypeRef for any that the top-down walker didn't reach (e.g., inside
    // interpolated expressions, complex type projections, or error subtrees).
    scan_all_type_refs(root, src, &mut refs);

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

            // Abstract method declaration in trait/class (no body).
            "function_declaration" => {
                let idx = push_function_def(&child, src, scope_tree, symbols, parent_index);
                // Extract TypeRef from return type and parameter types in declarations.
                if let Some(sym_idx) = idx {
                    extract_type_refs_from_function(&child, src, sym_idx, refs);
                }
            }

            "function_definition" => {
                let idx = push_function_def(&child, src, scope_tree, symbols, parent_index);
                if let Some(sym_idx) = idx {
                    extract_decorators(&child, src, sym_idx, refs);
                    // Extract TypeRef from return type and parameter types.
                    extract_type_refs_from_function(&child, src, sym_idx, refs);
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
                        // Recurse into the body with extract_node so that nested val/var/def
                        // definitions inside blocks are extracted as symbols, and infix/call
                        // expressions in deeply-nested blocks are visited.
                        extract_node(body, src, scope_tree, symbols, refs, Some(sym_idx));
                    }
                }
            }

            "val_definition" | "var_definition" | "val_declaration" | "var_declaration" => {
                // Extract type annotation *before* pushing the symbol (so we have the right index).
                let sym_idx = if let Some(type_node) = child.child_by_field_name("type") {
                    // For declarations, use parent_index; for definitions, we'll use the symbol we just created.
                    let idx_to_use = match child.kind() {
                        "val_definition" | "var_definition" => symbols.len(), // Will be the index of the symbol we push below
                        _ => parent_index.unwrap_or(0), // For declarations
                    };
                    push_val_var(&child, src, scope_tree, symbols, parent_index);
                    extract_type_refs_from_type_node(&type_node, src, idx_to_use, refs);
                    idx_to_use
                } else {
                    let idx = symbols.len();
                    push_val_var(&child, src, scope_tree, symbols, parent_index);
                    idx
                };
                // Recurse into the value expression for nested val/var/def definitions
                // (e.g. `val x = { val inner = ...; inner }`) and call edges.
                if matches!(child.kind(), "val_definition" | "var_definition") {
                    if let Some(value_node) = child.child_by_field_name("value") {
                        extract_calls_from_body(&value_node, src, sym_idx, refs);
                        extract_node(value_node, src, scope_tree, symbols, refs, Some(sym_idx));
                    }
                }
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

            // `package foo.bar { ... }` — emit a Namespace symbol and recurse.
            // Also handles `package foo.bar` (no body) by emitting the symbol only.
            "package_clause" => {
                let pkg_idx = push_package_clause(&child, src, scope_tree, symbols, parent_index);
                let effective_parent = pkg_idx.or(parent_index);
                if let Some(body) = child.child_by_field_name("body") {
                    extract_node(body, src, scope_tree, symbols, refs, effective_parent);
                } else {
                    // Scala `package foo.bar` at top-level with no braces — the rest
                    // of the file is implicitly in scope; recurse treating siblings
                    // as children (caller handles this via the main loop).
                    let mut cc = child.walk();
                    for inner in child.children(&mut cc) {
                        match inner.kind() {
                            "class_definition" | "object_definition" | "trait_definition"
                            | "enum_definition" | "function_definition" | "function_declaration"
                            | "val_definition" | "var_definition" | "import_declaration" => {
                                extract_node(inner, src, scope_tree, symbols, refs, effective_parent);
                            }
                            _ => {}
                        }
                    }
                }
            }

            // `export foo._` / `export foo.{Bar, Baz}` — emit Imports refs.
            "export_declaration" => {
                push_export(&child, src, symbols.len(), refs);
            }

            // extends_clause and with_clause are handled by extract_extends_with
            // when processing the parent class/trait/object/enum.  When they appear
            // as children of any other node (e.g. in a nested class inside a function
            // body), fall through to explicit type-ref walking so no edges are missed.
            "extends_clause" | "with_clause" => {
                if let Some(sym_idx) = parent_index {
                    super::symbols::extract_extends_with_node(&child, src, sym_idx, refs);
                }
            }

            "match_expression" => {
                if let Some(sym_idx) = parent_index {
                    extract_match_patterns(&child, src, sym_idx, refs);
                }
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            // for-expression / for-comprehension — extract embedded calls and type refs.
            "for_expression" => {
                if let Some(sym_idx) = parent_index {
                    extract_calls_from_body(&child, src, sym_idx, refs);
                }
                extract_node(child, src, scope_tree, symbols, refs, parent_index);
            }

            // Call expressions outside a function body (e.g. val/var initializers,
            // top-level statements, object body expressions).
            "call_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                dispatch_body_node(child, src, sym_idx, refs);
                extract_calls_from_body(&child, src, sym_idx, refs);
            }

            "infix_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                dispatch_body_node(child, src, sym_idx, refs);
                extract_calls_from_body(&child, src, sym_idx, refs);
            }

            // `new Dog(args)` at expression level
            "instance_expression" => {
                let sym_idx = parent_index.unwrap_or(0);
                extract_calls_from_body(&child, src, sym_idx, refs);
            }

            // Generic type arguments appearing in expression context (e.g. method call
            // type parameters, or a generic type used as a value).  Walk them for
            // nested type_identifier nodes.
            "type_arguments" => {
                let sym_idx = parent_index.unwrap_or(0);
                extract_type_refs_from_type_node(&child, src, sym_idx, refs);
            }

            // A bare type_identifier in expression context (e.g. pattern matching,
            // companion object reference, generic type position).
            "type_identifier" => {
                let sym_idx = parent_index.unwrap_or(0);
                let name = helpers::node_text(child, src);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: name,
                        kind: crate::types::EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
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
// Type reference extraction helpers
// ---------------------------------------------------------------------------

/// Extract TypeRef edges from a type annotation node (e.g., the `: String` part).
/// Recursively handles generic_type, compound_type, etc.
/// NOTE: We extract ALL type identifiers, including builtins. Filtering happens in resolution.
fn extract_type_refs_from_type_node(
    type_node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Handle the type_node itself if it's a type_identifier.
    if type_node.kind() == "type_identifier" {
        let name = helpers::node_text(*type_node, src);
        if !name.is_empty() {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: name,
                kind: crate::types::EdgeKind::TypeRef,
                line: type_node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
        return;
    }

    let mut cursor = type_node.walk();
    for child in type_node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" => {
                let name = helpers::node_text(child, src);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: name,
                        kind: crate::types::EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
            "generic_type" => {
                // Recurse into generic_type to find type_identifier and type_arguments.
                extract_type_refs_from_type_node(&child, src, source_symbol_index, refs);
            }
            "type_arguments" => {
                // Recurse into type arguments (e.g., `List[User]` → process `User`).
                extract_type_refs_from_type_node(&child, src, source_symbol_index, refs);
            }
            "compound_type" | "annotated_type" | "with_type" => {
                // Recurse into compound types.
                extract_type_refs_from_type_node(&child, src, source_symbol_index, refs);
            }
            "function_type" => {
                // Function types may have parameter and return type nodes.
                extract_type_refs_from_type_node(&child, src, source_symbol_index, refs);
            }
            _ => {
                // Recurse into other node types to find nested type_identifier nodes.
                extract_type_refs_from_type_node(&child, src, source_symbol_index, refs);
            }
        }
    }
}

/// Extract TypeRef edges from function parameter and return types.
fn extract_type_refs_from_function(
    func_node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Extract return type.
    if let Some(ret_type) = func_node.child_by_field_name("return_type") {
        extract_type_refs_from_type_node(&ret_type, src, source_symbol_index, refs);
    }

    // Extract parameter types.
    if let Some(params) = func_node.child_by_field_name("parameters") {
        extract_type_refs_from_type_node(&params, src, source_symbol_index, refs);
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
        // `new Dog(args)` as a direct function body expression.
        "instance_expression" => {
            let mut ic = node.walk();
            for inner in node.children(&mut ic) {
                match inner.kind() {
                    "type_identifier" => {
                        let name = node_text(inner, src);
                        if !name.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: name,
                                kind: crate::types::EdgeKind::Calls,
                                line: inner.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        }
                    }
                    "stable_type_identifier" => {
                        let full = node_text(inner, src);
                        let simple = full.rsplit('.').next().unwrap_or(&full).to_string();
                        if !simple.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: simple,
                                kind: crate::types::EdgeKind::Calls,
                                line: inner.start_position().row as u32,
                                module: Some(full),
                                chain: None,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Post-traversal full-tree type reference scan
// ---------------------------------------------------------------------------

/// Walk the entire CST and emit TypeRef edges for every `type_identifier` node
/// found. This catches type references that the top-down walker misses due to
/// structural gaps (e.g., type parameters in nested positions, string
/// interpolation types, or any node kind not explicitly handled above).
///
/// Deduplication is handled at resolution — over-emitting is always safe.
fn scan_all_type_refs(node: tree_sitter::Node, src: &[u8], refs: &mut Vec<ExtractedRef>) {
    scan_type_refs_inner(node, src, 0, refs);
}

fn scan_type_refs_inner(
    node: tree_sitter::Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() == "type_identifier" {
        let name = helpers::node_text(node, src);
        if !name.is_empty() && !super::builtins::is_scala_builtin(&name) {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: name,
                kind: crate::types::EdgeKind::TypeRef,
                line: node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
        // type_identifier is a leaf — no children to recurse into.
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        scan_type_refs_inner(child, src, source_symbol_index, refs);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

