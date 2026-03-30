// =============================================================================
// python/symbols.rs  —  Symbol extractors for Python
// =============================================================================

use super::calls::{build_chain, extract_calls_from_body};
use super::helpers::{
    detect_python_visibility, extract_docstring, extract_function_signature,
    extract_python_type_name, is_test_function, node_text, qualify, scope_from_prefix,
};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

pub(super) fn extract_function_definition(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
    decorators: &[String],
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_python_visibility(&name);

    let has_property = decorators.iter().any(|d| d == "property");
    let has_test_decorator = decorators.iter().any(|d| {
        d.starts_with("pytest.mark") || d == "test" || d.starts_with("pytest.fixture")
    });

    let kind = if has_property {
        SymbolKind::Property
    } else if name == "__init__" {
        SymbolKind::Constructor
    } else if is_test_function(&name, has_test_decorator) {
        SymbolKind::Test
    } else if inside_class {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    let body = node.child_by_field_name("body");
    let doc_comment = body.as_ref().and_then(|b| extract_docstring(b, source));
    let signature = extract_function_signature(node, source);

    let qualified_name_str = qualified_name.clone();
    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    if let Some(params) = node.child_by_field_name("parameters") {
        extract_python_typed_params_as_symbols(&params, source, symbols, refs, Some(idx), &qualified_name_str);
    }

    if let Some(body_node) = body {
        extract_calls_from_body(&body_node, source, idx, refs);
    }
}

pub(super) fn extract_python_typed_params_as_symbols(
    params_node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    func_qualified_name: &str,
) {
    let mut cursor = params_node.walk();
    for child in params_node.children(&mut cursor) {
        let (name, type_node) = match child.kind() {
            "typed_parameter" => {
                let type_node = match child.child_by_field_name("type") {
                    Some(t) => t,
                    None => continue,
                };
                let name_node = (0..child.child_count())
                    .filter_map(|i| child.child(i))
                    .find(|c| c.kind() == "identifier");
                let name = match name_node {
                    Some(n) => node_text(&n, source),
                    None => continue,
                };
                (name, type_node)
            }
            "typed_default_parameter" => {
                let name_node = match child.child_by_field_name("name") {
                    Some(n) => n,
                    None => continue,
                };
                let type_node = match child.child_by_field_name("type") {
                    Some(t) => t,
                    None => continue,
                };
                (node_text(&name_node, source), type_node)
            }
            _ => continue,
        };

        if name == "self" || name == "cls" {
            continue;
        }

        let type_name = extract_python_type_name(&type_node, source);
        if type_name.is_empty() {
            continue;
        }

        let qualified_name = qualify(&name, func_qualified_name);
        let scope_path = Some(func_qualified_name.to_string());

        let param_idx = symbols.len();
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name,
            kind: SymbolKind::Property,
            visibility: None,
            start_line: child.start_position().row as u32,
            end_line: child.end_position().row as u32,
            start_col: child.start_position().column as u32,
            end_col: child.end_position().column as u32,
            signature: Some(format!("{name}: {type_name}")),
            doc_comment: None,
            scope_path,
            parent_index,
        });

        refs.push(ExtractedRef {
            source_symbol_index: param_idx,
            target_name: type_name,
            kind: EdgeKind::TypeRef,
            line: type_node.start_position().row as u32,
            module: None,
            chain: None,
        });
    }
}

pub(super) fn extract_class_definition(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let name = node_text(&name_node, source);
    let qualified_name = qualify(&name, qualified_prefix);
    let visibility = detect_python_visibility(&name);

    let body = node.child_by_field_name("body");
    let doc_comment = body.as_ref().and_then(|b| extract_docstring(b, source));

    let signature = {
        let text = node_text(node, source);
        text.lines()
            .next()
            .map(|l| l.trim_end_matches(':').trim().to_string())
    };

    let idx = symbols.len();

    let new_prefix = if qualified_prefix.is_empty() {
        name.clone()
    } else {
        format!("{}.{}", qualified_prefix, name)
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Class,
        visibility,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature,
        doc_comment,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    if let Some(superclasses) = node.child_by_field_name("superclasses") {
        extract_superclass_refs(&superclasses, source, refs, idx);
    }

    if let Some(body_node) = body {
        super::extract_from_node(body_node, source, symbols, refs, Some(idx), &new_prefix, true);
    }
}

fn extract_superclass_refs(
    argument_list: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    class_idx: usize,
) {
    let mut cursor = argument_list.walk();
    for child in argument_list.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = node_text(&child, source);
                refs.push(ExtractedRef {
                    source_symbol_index: class_idx,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: child.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
            "attribute" => {
                if let Some(attr) = child.child_by_field_name("attribute") {
                    let name = node_text(&attr, source);
                    let obj = child
                        .child_by_field_name("object")
                        .map(|o| node_text(&o, source));
                    refs.push(ExtractedRef {
                        source_symbol_index: class_idx,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: obj,
                        chain: None,
                    });
                }
            }
            _ => {}
        }
    }
}

pub(super) fn extract_decorated_definition(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
) {
    let decorators = extract_decorator_names(node, source);

    // The symbol pushed by the inner call will land at this index.
    let symbol_index = symbols.len();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_definition" => {
                extract_function_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    inside_class,
                    &decorators,
                );
                super::decorators::extract_decorators(node, source, symbol_index, refs);
            }
            "class_definition" => {
                extract_class_definition(&child, source, symbols, refs, parent_index, qualified_prefix);
                super::decorators::extract_decorators(node, source, symbol_index, refs);
            }
            _ => {}
        }
    }
}

fn extract_decorator_names(node: &Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "decorator" {
            let mut dcursor = child.walk();
            for dchild in child.children(&mut dcursor) {
                match dchild.kind() {
                    "identifier" => {
                        names.push(node_text(&dchild, source));
                    }
                    "attribute" => {
                        names.push(node_text(&dchild, source));
                    }
                    "call" => {
                        if let Some(func) = dchild.child_by_field_name("function") {
                            names.push(node_text(&func, source));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    names
}

pub(super) fn extract_assignment_if_any(
    expr_stmt: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    inside_class: bool,
) {
    let mut cursor = expr_stmt.walk();
    for child in expr_stmt.children(&mut cursor) {
        if child.kind() == "assignment" {
            extract_assignment_node(&child, source, symbols, parent_index, qualified_prefix, inside_class);
        }
    }
}

fn extract_assignment_node(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
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
            push_variable_symbol(node, &left, &name, kind, symbols, parent_index, qualified_prefix);
        }
        "pattern_list" | "tuple_pattern" => {
            let mut cursor = left.walk();
            for elem in left.children(&mut cursor) {
                if elem.kind() == "identifier" {
                    let name = node_text(&elem, source);
                    let kind = classify_assignment_name(&name, inside_class);
                    push_variable_symbol(node, &elem, &name, kind, symbols, parent_index, qualified_prefix);
                }
            }
        }
        _ => {}
    }
}

fn classify_assignment_name(name: &str, _inside_class: bool) -> SymbolKind {
    let stripped = name.trim_start_matches('_');
    let _ = stripped; // all assignments → Variable in this codebase
    SymbolKind::Variable
}

#[allow(clippy::too_many_arguments)]
fn push_variable_symbol(
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
    });
}

