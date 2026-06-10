// =============================================================================
// python/assignments.rs — assignment-driven symbol/variable extraction
// =============================================================================

use super::calls::{build_chain, extract_calls_from_body};
use super::helpers::{detect_python_visibility, node_text, qualify, scope_from_prefix};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use std::collections::HashMap;
use tree_sitter::Node;

pub(super) fn extract_assignment_if_any(
    expr_stmt: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
) {
    let mut cursor = expr_stmt.walk();
    for child in expr_stmt.children(&mut cursor) {
        if child.kind() == "assignment" {
            extract_assignment_node(
                &child,
                source,
                symbols,
                refs,
                parent_index,
                qualified_prefix,
                inside_class,
            );
        }
    }
}

fn extract_assignment_node(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
) {
    let left = match node.child_by_field_name("left") {
        Some(n) => n,
        None => return,
    };

    match left.kind() {
        "identifier" => {
            let name = node_text(&left, source);
            let kind = classify_assignment_name(&name, inside_class);
            let sym_idx = symbols.len();
            push_variable_symbol(
                node,
                &left,
                &name,
                kind,
                symbols,
                parent_index,
                qualified_prefix,
            );
            // Infer the variable type from the RHS when no explicit annotation.
            // `repo = UserRepository(db)` — first positional RHS is the `right` field.
            if let Some(rhs) = node.child_by_field_name("right") {
                infer_python_variable_type(&rhs, source, sym_idx, refs);
            }
        }
        // `self.x = value` — attribute assignment defining an instance field.
        // Extract the attribute name (last component after '.').
        "attribute" => {
            if let Some(attr_node) = left.child_by_field_name("attribute") {
                let name = node_text(&attr_node, source);
                if !name.is_empty() {
                    let sym_idx = symbols.len();
                    push_variable_symbol(
                        node,
                        &attr_node,
                        &name,
                        SymbolKind::Variable,
                        symbols,
                        parent_index,
                        qualified_prefix,
                    );
                    if let Some(rhs) = node.child_by_field_name("right") {
                        infer_python_variable_type(&rhs, source, sym_idx, refs);
                    }
                }
            }
        }
        "pattern_list" | "tuple_pattern" => {
            // For tuple unpacking we don't attempt RHS inference (ambiguous mapping).
            let mut cursor = left.walk();
            for elem in left.children(&mut cursor) {
                if elem.kind() == "identifier" {
                    let name = node_text(&elem, source);
                    let kind = classify_assignment_name(&name, inside_class);
                    push_variable_symbol(
                        node,
                        &elem,
                        &name,
                        kind,
                        symbols,
                        parent_index,
                        qualified_prefix,
                    );
                }
            }
        }
        _ => {}
    }
}

/// Walk a call-expression's function node and return the leaf identifier name
/// when the receiver is itself a constructor call. Handles
/// `Path(...).resolve()` → `"Path"`, `Foo().bar().baz()` → `"Foo"`. Returns
/// `None` for plain attribute / identifier nodes so the caller falls back to
/// `node_text` for those.
fn inner_call_function_name(node: &tree_sitter::Node, source: &str) -> Option<String> {
    if node.kind() != "call" {
        return None;
    }
    let func = node.child_by_field_name("function")?;
    match func.kind() {
        "identifier" => Some(node_text(&func, source)),
        "attribute" => func
            .child_by_field_name("attribute")
            .map(|a| node_text(&a, source)),
        "call" => inner_call_function_name(&func, source),
        _ => None,
    }
}

/// Inspect the Python RHS expression and emit a TypeRef when the type can be
/// determined heuristically:
///
/// - `Foo(args)` — `call` whose function is an uppercase identifier → TypeRef "Foo"
/// - `Foo.create(args)` — `call` whose function is `attribute` with uppercase object
///   → TypeRef "Foo"
///
/// Explicitly annotated variables (`x: Foo = ...`) are handled elsewhere via the
/// type-annotation extractor.
fn infer_python_variable_type(
    rhs: &tree_sitter::Node,
    source: &str,
    var_sym_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if rhs.kind() != "call" {
        return;
    }
    let func_node = match rhs.child_by_field_name("function") {
        Some(n) => n,
        None => return,
    };
    let type_name = match func_node.kind() {
        // `Foo(args)` — direct constructor call
        "identifier" => {
            let name = node_text(&func_node, source);
            if name
                .chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false)
            {
                name
            } else {
                return;
            }
        }
        // `Foo.create(args)` or `module.Foo(args)` — attribute call
        "attribute" => {
            let object = match func_node.child_by_field_name("object") {
                Some(n) => n,
                None => return,
            };
            // Skip when the receiver is itself a call expression
            // (`Path(tempfile.mkdtemp()).resolve()`): the outer chain
            // would otherwise emit `Path(tempfile.mkdtemp())` as a
            // type name. Walk into the inner call's function instead.
            let resolved_obj = inner_call_function_name(&object, source);
            let obj = resolved_obj.unwrap_or_else(|| node_text(&object, source));
            // Only emit if the object name starts uppercase (class factory pattern).
            if obj
                .chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false)
            {
                obj
            } else {
                return;
            }
        }
        _ => return,
    };

    refs.push(ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: var_sym_idx,
        target_name: type_name,
        kind: EdgeKind::TypeRef,
        line: rhs.start_position().row as u32,
        col: 0,
        module: None,
        chain: None,
        byte_offset: rhs.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

fn classify_assignment_name(name: &str, _inside_class: bool) -> SymbolKind {
    let stripped = name.trim_start_matches('_');
    let _ = stripped; // all assignments -> Variable in this codebase
    SymbolKind::Variable
}

pub(super) fn push_variable_symbol(
    node: &Node,
    name_node: &Node,
    name: &str,
    kind: SymbolKind,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let qualified_name = qualify(name, qualified_prefix);
    let visibility = detect_python_visibility(name);

    symbols.push(ExtractedSymbol {
        name: name.to_string(),
        qualified_name,
        kind,
        visibility,
        start_line: name_node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: name_node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });
}

pub(super) fn extract_augmented_assignment(
    node: &Node,
    source: &str,
    enclosing_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    import_map: &HashMap<String, String>,
) {
    // Recurse for any calls in the right-hand side.
    extract_calls_from_body(node, source, enclosing_symbol_index, refs, import_map);

    // Emit a Calls edge if the left side is an attribute access (member access).
    if let Some(left) = node.child_by_field_name("left") {
        if left.kind() == "attribute" {
            if let Some(chain) = build_chain(&left, source) {
                let target = chain
                    .segments
                    .last()
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                if !target.is_empty() {
                    refs.push(ExtractedRef {
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index: enclosing_symbol_index,
                        target_name: target,
                        kind: EdgeKind::Calls,
                        line: left.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: Some(chain),
                        byte_offset: left.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
            }
        }
    }
}
