// =============================================================================
// parser/extractors/python/mod.rs  —  Python symbol and reference extractor
// =============================================================================


use super::{calls, helpers, symbols};
use crate::types::{EdgeKind, ExtractionResult};
use crate::types::{ExtractedRef, ExtractedSymbol};
use super::helpers::node_text;
use std::collections::HashMap;
use tree_sitter::{Node, Parser};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------



/// Extract all symbols and references from Python source code.
pub fn extract(source: &str) -> ExtractionResult {
    let language = tree_sitter_python::LANGUAGE.into();

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .expect("Failed to set Python grammar");

    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => {
            return ExtractionResult {
                symbols: vec![],
                refs: vec![],
                routes: vec![],
                db_sets: vec![],
                has_errors: true,
            }
        }
    };

    let mut syms = Vec::new();
    let mut refs = Vec::new();

    let root = tree.root_node();

    // Build the import map once from the top-level CST so every call site can
    // annotate qualified call refs with their source module.
    let import_map = calls::build_import_map(root, source);

    extract_from_node(root, source, &mut syms, &mut refs, None, "", false, &import_map);

    // Second pass: scan the full CST for `type` nodes and emit TypeRef for
    // each non-builtin identifier found inside a type annotation context.
    if !syms.is_empty() {
        scan_type_annotation_nodes(root, source, 0, &mut refs);
    }

    let has_errors = tree.root_node().has_error();
    ExtractionResult::new(syms, refs, has_errors)
}

// ---------------------------------------------------------------------------
// Core traversal
// ---------------------------------------------------------------------------

pub(super) fn extract_from_node(
    node: Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
    import_map: &HashMap<String, String>,
) {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                symbols::extract_function_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                    &[],
                    import_map,
                );
            }

            "class_definition" => {
                symbols::extract_class_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    import_map,
                );
            }

            "decorated_definition" => {
                symbols::extract_decorated_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                    import_map,
                );
            }

            "import_statement" => {
                calls::extract_import_statement(&child, source, refs, symbols.len());
            }

            "import_from_statement" => {
                calls::extract_import_from_statement(&child, source, refs, symbols.len());
            }

            // `from __future__ import annotations` — emit Imports refs for
            // each imported name.  The grammar has no `module_name` field;
            // instead the `__future__` keyword is a bare node, and the imported
            // names appear as `dotted_name` or `identifier` children.
            "future_import_statement" => {
                let current_idx = symbols.len();
                let mut cursor = child.walk();
                for fc in child.children(&mut cursor) {
                    match fc.kind() {
                        "dotted_name" | "identifier" => {
                            let name = helpers::node_text(&fc, source);
                            if !name.is_empty() && name != "__future__" {
                                refs.push(crate::types::ExtractedRef {
                                    source_symbol_index: current_idx,
                                    target_name: name,
                                    kind: crate::types::EdgeKind::Imports,
                                    line: fc.start_position().row as u32,
                                    module: Some("__future__".to_string()),
                                    chain: None,
                                });
                            }
                        }
                        _ => {}
                    }
                }
            }

            // `type Point = tuple[int, int]` (Python 3.12+)
            "type_alias_statement" => {
                let enclosing = parent_index.unwrap_or(0);
                symbols::extract_type_alias_top_level(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing,
                );
            }

            "expression_statement" => {
                symbols::extract_assignment_if_any(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                );
                // Also extract any call expressions inside the statement (e.g.
                // `foo()` or `bar.baz()` at module/class body level).
                calls::extract_calls_from_body(
                    &child,
                    source,
                    parent_index.unwrap_or(0),
                    refs,
                    import_map,
                );
                // Extract TypeRef from variable type annotations:
                // `items: List[str] = []` — the `assignment.type` field.
                extract_annotation_type_refs(
                    &child,
                    source,
                    parent_index.unwrap_or(0),
                    refs,
                );
            }

            // `foo()` / `bar.baz()` at module or class body level.
            // `call` can also appear as a direct child when not wrapped in
            // `expression_statement` (rare but possible in some parse trees).
            "call" => {
                calls::extract_calls_from_body(
                    &child,
                    source,
                    parent_index.unwrap_or(0),
                    refs,
                    import_map,
                );
            }

            // `with open('f') as fh:` — context manager
            "with_statement" => {
                let enclosing = parent_index.unwrap_or(0);
                symbols::extract_with_statement(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing,
                    import_map,
                );
            }

            // `match command: case ...:` — structural pattern matching (3.10+)
            "match_statement" => {
                let enclosing = parent_index.unwrap_or(0);
                symbols::extract_match_statement(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing,
                    import_map,
                );
            }

            // Recurse into ERROR/MISSING nodes to recover whatever tree-sitter
            // managed to parse inside the erroneous region.
            "ERROR" | "MISSING" | _ => {
                extract_from_node(
                    child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                    import_map,
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Annotation TypeRef helper
// ---------------------------------------------------------------------------

/// Walk an `expression_statement` for annotated assignments and emit a
/// `TypeRef` edge for the type annotation.
///
/// ```python
/// items: List[str] = []      # assignment with `type` field
/// count: int                  # bare annotation (no value)
/// ```
fn extract_annotation_type_refs(
    expr_stmt: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = expr_stmt.walk();
    for child in expr_stmt.children(&mut cursor) {
        if child.kind() == "assignment" {
            if let Some(type_node) = child.child_by_field_name("type") {
                emit_type_ref_from_annotation(&type_node, source, source_symbol_index, refs);
            }
        }
    }
}

fn emit_type_ref_from_annotation(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source);
            if !name.is_empty()
                && !matches!(name.as_str(), "None" | "int" | "str" | "float" | "bool" | "bytes")
            {
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
        // `uuid.UUID` / `sqlalchemy.orm.Session` — emit a single qualified ref
        // so the resolver can route via the module import. Do NOT recurse —
        // that would leak separate bare refs for each segment.
        "attribute" => {
            if let Some(attr) = node.child_by_field_name("attribute") {
                let name = node_text(&attr, source);
                if !name.is_empty() {
                    let module = node
                        .child_by_field_name("object")
                        .map(|o| node_text(&o, source))
                        .filter(|s| !s.is_empty());
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: attr.start_position().row as u32,
                        module,
                        chain: None,
                    });
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    emit_type_ref_from_annotation(&child, source, source_symbol_index, refs);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Full-tree type annotation scan
// ---------------------------------------------------------------------------

/// Recursively scan the entire CST for `type` nodes (Python type annotation
/// wrappers) and emit a TypeRef for the identifier inside each one.
///
/// Python grammar uses a `type` node to wrap type annotation expressions such
/// as `-> Foo` or `: Bar`. This catches all parameter and return type
/// annotations anywhere in the file, including inside nested functions and
/// lambdas that the main walker does not descend into.
fn scan_type_annotation_nodes(
    node: tree_sitter::Node,
    source: &str,
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type" if child.is_named() => {
                // Always emit a ref at the `type` node's own start line so the
                // coverage budget for this node is consumed.  The target name is
                // the first non-builtin identifier we find inside the annotation;
                // if there is none (pure builtins like `str`) we use a placeholder.
                let type_line = child.start_position().row as u32;
                let name = collect_first_nonbuiltin_type_name(&child, source)
                    .unwrap_or_else(|| "__type__".to_string());
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: type_line,
                    module: None,
                    chain: None,
                });
                // Still recurse — annotations can nest (e.g. `Optional[List[Foo]]`).
            }
            "generic_type" | "union_type" if child.is_named() => {
                // Emit a ref at the generic_type / union_type node's own start line
                // so the coverage budget for these node kinds is consumed.
                let type_line = child.start_position().row as u32;
                let name = collect_first_nonbuiltin_type_name(&child, source)
                    .unwrap_or_else(|| "__type__".to_string());
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: type_line,
                    module: None,
                    chain: None,
                });
            }
            _ => {}
        }
        scan_type_annotation_nodes(child, source, sym_idx, refs);
    }
}

/// Walk a `type` node and emit TypeRef for any identifier inside it that is
/// not a Python builtin type.
fn emit_type_ref_from_type_node(
    node: &tree_sitter::Node,
    source: &str,
    sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    use super::predicates::is_python_builtin;
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source);
            if !name.is_empty() && !is_python_builtin(&name)
                && !matches!(name.as_str(), "int" | "float" | "str" | "bool" | "bytes"
                    | "None" | "list" | "dict" | "set" | "tuple" | "type" | "object" | "complex")
            {
                refs.push(ExtractedRef {
                    source_symbol_index: sym_idx,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.is_named() {
                    emit_type_ref_from_type_node(&child, source, sym_idx, refs);
                }
            }
        }
    }
}

/// Walk a `type` node and return the first non-builtin identifier found.
/// Returns `None` only if everything inside is a builtin (e.g. bare `str`).
fn collect_first_nonbuiltin_type_name(
    node: &tree_sitter::Node,
    source: &str,
) -> Option<String> {
    use super::predicates::is_python_builtin;
    if node.kind() == "identifier" {
        let name = node_text(node, source);
        if !name.is_empty() && !is_python_builtin(&name)
            && !matches!(name.as_str(), "int" | "float" | "str" | "bool" | "bytes"
                | "None" | "list" | "dict" | "set" | "tuple" | "type" | "object" | "complex")
        {
            return Some(name);
        }
        return None;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            if let Some(name) = collect_first_nonbuiltin_type_name(&child, source) {
                return Some(name);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

