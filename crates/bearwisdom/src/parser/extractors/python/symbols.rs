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
        extract_python_typed_params_as_symbols(
            &params,
            source,
            symbols,
            refs,
            Some(idx),
            &qualified_name_str,
        );
    }

    if let Some(body_node) = body {
        extract_calls_from_body(&body_node, source, idx, refs);
        // Walk body for constructs that emit Variable symbols in addition to calls.
        extract_body_symbols(
            &body_node,
            source,
            symbols,
            refs,
            Some(idx),
            &qualified_name_str,
            idx,
        );
    }
}

/// Walk a function/method body to emit Variable symbols for constructs that
/// tree-sitter surfaces as sub-expressions rather than statements.
///
/// Covers:
///   - `with_statement` -> alias variable + chain TypeRef
///   - `match_statement` -> pattern capture variables + class TypeRefs
///   - `named_expression` (walrus `:=`) -> variable + chain TypeRef
///   - `list/dict/set_comprehension` / `generator_expression` -> loop variable
///   - `lambda` -> parameter variables
///
/// Note: call extraction is already handled by `extract_calls_from_body`; this
/// function only handles the symbol-emitting side.
fn extract_body_symbols(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_idx: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "with_statement" => {
                extract_with_statement(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                );
            }
            "match_statement" => {
                extract_match_statement(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                );
            }
            "named_expression" => {
                extract_named_expression(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                );
            }
            "list_comprehension"
            | "dictionary_comprehension"
            | "set_comprehension"
            | "generator_expression" => {
                extract_comprehension(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                );
            }
            "lambda" => {
                extract_lambda(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                );
            }
            "f_string" | "fstring" => {
                extract_fstring_calls(&child, source, enclosing_idx, refs);
            }
            _ => {
                extract_body_symbols(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                );
            }
        }
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
                extract_class_definition(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                );
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
            extract_assignment_node(
                &child,
                source,
                symbols,
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
            push_variable_symbol(
                node,
                &left,
                &name,
                kind,
                symbols,
                parent_index,
                qualified_prefix,
            );
        }
        "pattern_list" | "tuple_pattern" => {
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

fn classify_assignment_name(name: &str, _inside_class: bool) -> SymbolKind {
    let stripped = name.trim_start_matches('_');
    let _ = stripped; // all assignments -> Variable in this codebase
    SymbolKind::Variable
}

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

// =============================================================================
// With statements / context managers
// =============================================================================

/// Extract `with open('f') as fh:` and `with db.session() as session:`.
///
/// Tree-sitter-python shape:
/// ```text
/// with_statement
///   with_clause
///     with_item
///       value: call / identifier / attribute
///       as: identifier   <- alias (optional)
///   block
/// ```
pub(super) fn extract_with_statement(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "with_clause" => {
                let mut ic = child.walk();
                for item in child.children(&mut ic) {
                    if item.kind() == "with_item" {
                        extract_with_item(
                            &item,
                            source,
                            symbols,
                            refs,
                            parent_index,
                            qualified_prefix,
                            enclosing_symbol_index,
                        );
                    }
                }
            }
            // Some grammar versions place with_item directly under with_statement.
            "with_item" => {
                extract_with_item(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_symbol_index,
                );
            }
            "block" => {
                extract_body_symbols(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_symbol_index,
                );
            }
            _ => {}
        }
    }
}

fn extract_with_item(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
) {
    // In tree-sitter-python 0.25, `with open('f') as f:` is represented as:
    //
    //   with_item
    //     as_pattern            ← the value field IS the as_pattern
    //       call                ← open('f')
    //       as
    //       as_pattern_target   ← "f"  (via alias field)
    //
    // Without an alias the value field is simply the expression (call, identifier, etc.).
    let value_node = node.child_by_field_name("value").or_else(|| node.named_child(0));

    let (cm_expr, alias_ident) = match &value_node {
        Some(v) if v.kind() == "as_pattern" => {
            // The context manager expression is the first non-punctuation named child.
            let expr = v.named_child(0);
            // The alias identifier is inside as_pattern_target, accessed via the alias field.
            let alias = v.child_by_field_name("alias").and_then(|t| {
                // as_pattern_target wraps the identifier
                if t.kind() == "as_pattern_target" {
                    t.named_child(0)
                } else if t.kind() == "identifier" {
                    Some(t)
                } else {
                    None
                }
            });
            (expr, alias)
        }
        other => (other.as_ref().copied(), None),
    };

    // Emit calls from the context manager expression.
    if let Some(ref expr) = cm_expr {
        extract_calls_from_body(expr, source, enclosing_symbol_index, refs);
    }

    // Emit alias Variable and chain TypeRef when the cm expression is a call.
    if let Some(alias) = alias_ident {
        let name = node_text(&alias, source);
        let sym_idx = symbols.len();
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: qualify(&name, qualified_prefix),
            kind: SymbolKind::Variable,
            visibility: detect_python_visibility(&name),
            start_line: alias.start_position().row as u32,
            end_line: alias.end_position().row as u32,
            start_col: alias.start_position().column as u32,
            end_col: alias.end_position().column as u32,
            signature: Some(format!("with ... as {name}")),
            doc_comment: None,
            scope_path: scope_from_prefix(qualified_prefix),
            parent_index,
        });

        if let Some(expr) = cm_expr {
            if expr.kind() == "call" {
                if let Some(func) = expr.child_by_field_name("function") {
                    if let Some(chain) = build_chain(&func, source) {
                        let target = chain
                            .segments
                            .last()
                            .map(|s| s.name.clone())
                            .unwrap_or_default();
                        if !target.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index: sym_idx,
                                target_name: target,
                                kind: EdgeKind::TypeRef,
                                line: expr.start_position().row as u32,
                                module: None,
                                chain: Some(chain),
                            });
                        }
                    }
                }
            }
        }
    }
}

// =============================================================================
// Comprehensions (list, dict, set, generator)
// =============================================================================

/// Extract calls and loop-variable symbols from comprehension expressions.
///
/// Handles `list_comprehension`, `dictionary_comprehension`, `set_comprehension`,
/// and `generator_expression`.
pub(super) fn extract_comprehension(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "for_in_clause" => {
                if let Some(left) = child.child_by_field_name("left") {
                    extract_for_in_vars(
                        &left,
                        source,
                        symbols,
                        parent_index,
                        qualified_prefix,
                        &child,
                    );
                }
                if let Some(right) = child.child_by_field_name("right") {
                    extract_calls_from_body(&right, source, enclosing_symbol_index, refs);
                }
            }
            _ => {
                extract_calls_from_body(&child, source, enclosing_symbol_index, refs);
            }
        }
    }
}

fn extract_for_in_vars(
    left_node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    clause_node: &Node,
) {
    match left_node.kind() {
        "identifier" => {
            let name = node_text(left_node, source);
            if name != "_" {
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
                    qualified_name: qualify(&name, qualified_prefix),
                    kind: SymbolKind::Variable,
                    visibility: detect_python_visibility(&name),
                    start_line: left_node.start_position().row as u32,
                    end_line: clause_node.end_position().row as u32,
                    start_col: left_node.start_position().column as u32,
                    end_col: left_node.end_position().column as u32,
                    signature: None,
                    doc_comment: None,
                    scope_path: scope_from_prefix(qualified_prefix),
                    parent_index,
                });
            }
        }
        "pattern_list" | "tuple_pattern" => {
            let mut cursor = left_node.walk();
            for elem in left_node.children(&mut cursor) {
                if elem.kind() == "identifier" {
                    let name = node_text(&elem, source);
                    if name != "_" {
                        symbols.push(ExtractedSymbol {
                            name: name.clone(),
                            qualified_name: qualify(&name, qualified_prefix),
                            kind: SymbolKind::Variable,
                            visibility: detect_python_visibility(&name),
                            start_line: elem.start_position().row as u32,
                            end_line: clause_node.end_position().row as u32,
                            start_col: elem.start_position().column as u32,
                            end_col: elem.end_position().column as u32,
                            signature: None,
                            doc_comment: None,
                            scope_path: scope_from_prefix(qualified_prefix),
                            parent_index,
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

// =============================================================================
// Walrus operator (:=) -- named_expression
// =============================================================================

/// Extract `(user := find_user(id))` -- a `named_expression` node.
pub(super) fn extract_named_expression(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };
    let value_node = match node.child_by_field_name("value") {
        Some(n) => n,
        None => return,
    };

    let name = node_text(&name_node, source);

    let sym_idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: qualify(&name, qualified_prefix),
        kind: SymbolKind::Variable,
        visibility: detect_python_visibility(&name),
        start_line: name_node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: name_node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("{name} :=")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    extract_calls_from_body(&value_node, source, enclosing_symbol_index, refs);

    if value_node.kind() == "call" {
        if let Some(func) = value_node.child_by_field_name("function") {
            if let Some(chain) = build_chain(&func, source) {
                let target = chain.segments.last().map(|s| s.name.clone()).unwrap_or_default();
                if !target.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: sym_idx,
                        target_name: target,
                        kind: EdgeKind::TypeRef,
                        line: value_node.start_position().row as u32,
                        module: None,
                        chain: Some(chain),
                    });
                }
            }
        }
    }
}

// =============================================================================
// Match statement (Python 3.10+)
// =============================================================================

/// Extract type refs and pattern variables from a `match_statement`.
///
/// Tree-sitter-python 0.25 shape:
/// ```text
/// match_statement
///   "match"
///   <subject expression>
///   ":"
///   block                 ← case_clauses live inside this block
///     case_clause
///       "case"
///       case_pattern
///         <actual pattern>
///       ":"
///       block             ← case body
/// ```
pub(super) fn extract_match_statement(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "block" => {
                // The case_clauses are inside this block.
                let mut bc = child.walk();
                for clause in child.children(&mut bc) {
                    if clause.kind() == "case_clause" {
                        extract_case_clause(
                            &clause,
                            source,
                            symbols,
                            refs,
                            parent_index,
                            qualified_prefix,
                            enclosing_symbol_index,
                        );
                    }
                }
            }
            "case_clause" => {
                // Fallback for grammar versions where case_clauses are direct children.
                extract_case_clause(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_symbol_index,
                );
            }
            _ => {}
        }
    }
}

fn extract_case_clause(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
) {
    // In tree-sitter-python 0.25, case_clause structure:
    //
    //   case_clause
    //     "case"               ← keyword
    //     case_pattern         ← pattern wrapper (one or more)
    //       <actual pattern>   ← as_pattern | class_pattern | dotted_name | ...
    //     ":"
    //     block                ← consequence (via body field)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "block" => {
                extract_body_symbols(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_symbol_index,
                );
            }
            "case_pattern" => {
                // Descend one level: actual pattern is the single named child of case_pattern.
                if let Some(inner) = child.named_child(0) {
                    extract_pattern_refs(
                        &inner,
                        source,
                        symbols,
                        refs,
                        parent_index,
                        qualified_prefix,
                        enclosing_symbol_index,
                    );
                }
            }
            _ => {}
        }
    }
}

fn extract_pattern_refs(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
) {
    match node.kind() {
        // case_pattern is a transparent wrapper — recurse into its child.
        "case_pattern" => {
            if let Some(inner) = node.named_child(0) {
                extract_pattern_refs(
                    &inner,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_symbol_index,
                );
            }
        }

        // `User(name=n)` or `Admin()` — emit TypeRef for the class name.
        // class_pattern children: dotted_name (class), then case_pattern args.
        // No cls field in the grammar — first named child is the dotted_name.
        "class_pattern" => {
            // First named child is the dotted_name (e.g. "User" or "pkg.Admin").
            if let Some(class_node) = node.named_child(0) {
                // dotted_name contains identifiers; use the whole text as the type name.
                let class_name = node_text(&class_node, source);
                if !class_name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: enclosing_symbol_index,
                        target_name: class_name,
                        kind: EdgeKind::TypeRef,
                        line: class_node.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
            // Recurse into argument patterns for nested captures.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "case_pattern" || child.kind() == "keyword_pattern" {
                    extract_pattern_refs(
                        &child,
                        source,
                        symbols,
                        refs,
                        parent_index,
                        qualified_prefix,
                        enclosing_symbol_index,
                    );
                }
            }
        }

        // keyword_pattern: `name=n` inside class_pattern args.
        // Emit Variable for the bound identifier (second named child).
        "keyword_pattern" => {
            if let Some(binding) = node.named_child(1) {
                let name = node_text(&binding, source);
                if name != "_" && !name.is_empty() {
                    push_variable_symbol(
                        node,
                        &binding,
                        &name,
                        SymbolKind::Variable,
                        symbols,
                        parent_index,
                        qualified_prefix,
                    );
                }
            }
        }

        // `Admin() as admin` — recurse into inner pattern, emit Variable for alias.
        // In match context, as_pattern children: case_pattern + "as" + identifier.
        // (alias field points to as_pattern_target, but in match grammar it is just identifier)
        "as_pattern" => {
            // Recurse into the inner pattern (first named child, likely case_pattern).
            if let Some(inner) = node.named_child(0) {
                if inner.kind() != "as_pattern_target" && inner.kind() != "identifier" {
                    extract_pattern_refs(
                        &inner,
                        source,
                        symbols,
                        refs,
                        parent_index,
                        qualified_prefix,
                        enclosing_symbol_index,
                    );
                }
            }
            // The alias: check the alias field first (as_pattern_target), then scan for
            // a trailing identifier child after the "as" keyword.
            let alias_name = node
                .child_by_field_name("alias")
                .map(|t| {
                    // as_pattern_target may wrap an identifier
                    if t.kind() == "as_pattern_target" {
                        t.named_child(0)
                            .map(|n| node_text(&n, source))
                            .unwrap_or_else(|| node_text(&t, source))
                    } else {
                        node_text(&t, source)
                    }
                })
                .or_else(|| {
                    // Fallback: scan for identifier after "as" keyword token.
                    let mut saw_as = false;
                    let mut found = None;
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        if child.kind() == "as" {
                            saw_as = true;
                        } else if saw_as && child.kind() == "identifier" {
                            found = Some(node_text(&child, source));
                            break;
                        }
                    }
                    found
                });

            if let Some(name) = alias_name {
                if !name.is_empty() && name != "_" {
                    // Find the node to use as position anchor.
                    let pos_node = node.child_by_field_name("alias").unwrap_or(*node);
                    push_variable_symbol(
                        node,
                        &pos_node,
                        &name,
                        SymbolKind::Variable,
                        symbols,
                        parent_index,
                        qualified_prefix,
                    );
                }
            }
        }

        "capture_pattern" => {
            let name = node_text(node, source);
            if !name.is_empty() && name != "_" {
                push_variable_symbol(
                    node,
                    node,
                    &name,
                    SymbolKind::Variable,
                    symbols,
                    parent_index,
                    qualified_prefix,
                );
            }
        }

        "or_pattern" | "union_pattern" | "sequence_pattern" | "tuple_pattern"
        | "list_pattern" | "group_pattern" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_pattern_refs(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_symbol_index,
                );
            }
        }

        _ => {}
    }
}

// =============================================================================
// Lambda expressions
// =============================================================================

/// Extract calls and parameter symbols from a `lambda` expression.
pub(super) fn extract_lambda(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
) {
    if let Some(params) = node.child_by_field_name("parameters") {
        let mut cursor = params.walk();
        for param in params.children(&mut cursor) {
            let name = match param.kind() {
                "identifier" => node_text(&param, source),
                "default_parameter" | "typed_parameter" | "typed_default_parameter" => param
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, source))
                    .unwrap_or_default(),
                _ => continue,
            };
            if name.is_empty() || name == "self" || name == "cls" {
                continue;
            }
            symbols.push(ExtractedSymbol {
                name: name.clone(),
                qualified_name: qualify(&name, qualified_prefix),
                kind: SymbolKind::Variable,
                visibility: detect_python_visibility(&name),
                start_line: param.start_position().row as u32,
                end_line: param.end_position().row as u32,
                start_col: param.start_position().column as u32,
                end_col: param.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: scope_from_prefix(qualified_prefix),
                parent_index,
            });
        }
    }

    if let Some(body) = node.child_by_field_name("body") {
        extract_calls_from_body(&body, source, enclosing_symbol_index, refs);
    }
}

// =============================================================================
// F-string interpolation (low priority -- call extraction only)
// =============================================================================

/// Extract calls from f-string interpolation expressions.
pub(super) fn extract_fstring_calls(
    node: &Node,
    source: &str,
    enclosing_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "interpolation" || child.kind() == "fstring_expression" {
            let mut ic = child.walk();
            for expr in child.children(&mut ic) {
                if expr.is_named() {
                    extract_calls_from_body(&expr, source, enclosing_symbol_index, refs);
                }
            }
        }
    }
}
