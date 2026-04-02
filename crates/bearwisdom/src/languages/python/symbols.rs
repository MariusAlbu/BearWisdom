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

            // `async for item in stream:` — same shape as for_statement; recurse body.
            "async_for_statement" | "for_statement" => {
                extract_for_statement(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                );
            }

            // `async with session() as s:` — same logic as with_statement.
            "async_with_statement" => {
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

            // `global x` / `nonlocal y` — scope annotations; no symbols emitted,
            // but recurse so any calls inside unusual grammar structures are found.
            "global_statement" | "nonlocal_statement" => {}

            // `raise ValueError("msg")` — extract calls within + TypeRef for exception.
            "raise_statement" => {
                extract_raise_statement(&child, source, enclosing_idx, refs);
            }

            // `assert isinstance(x, Foo)` — extract calls within the test expression.
            "assert_statement" => {
                extract_calls_from_body(&child, source, enclosing_idx, refs);
            }

            // `del obj.field` — extract member access.
            "delete_statement" => {
                extract_calls_from_body(&child, source, enclosing_idx, refs);
            }

            // `x += 1` / `self.count += 1` — extract member access on left side.
            "augmented_assignment" => {
                extract_augmented_assignment(&child, source, enclosing_idx, refs);
            }

            // `type Point = tuple[int, int]` (Python 3.12+)
            "type_alias_statement" => {
                extract_type_alias(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                );
            }

            // `try: ... except ValueError as e:` — extract exception TypeRef and binding.
            "except_clause" | "except_group_clause" => {
                extract_except_clause(
                    &child,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_idx,
                );
            }

            // `try: ... finally: ...` — recurse body.
            "try_statement" => {
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

            // `yield value` / `yield from iter` — recurse for calls.
            "yield" | "yield_statement" | "yield_expression" => {
                extract_calls_from_body(&child, source, enclosing_idx, refs);
            }

            // Conditional expression: `a if cond else b` — recurse both branches.
            "conditional_expression" => {
                extract_calls_from_body(&child, source, enclosing_idx, refs);
            }

            // Structural/container expressions — recurse for calls.
            "tuple" | "list" | "dictionary" | "set" | "slice"
            | "parenthesized_expression" | "starred_expression"
            | "binary_operator" | "boolean_operator"
            | "comparison_operator" | "unary_operator"
            | "not_operator" | "await" => {
                extract_calls_from_body(&child, source, enclosing_idx, refs);
                // Also recurse for nested body-symbols (e.g. comprehensions inside tuples).
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
        match child.kind() {
            "typed_parameter" | "typed_default_parameter" => {
                let (name, type_node) = if child.kind() == "typed_parameter" {
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
                } else {
                    let name_node = match child.child_by_field_name("name") {
                        Some(n) => n,
                        None => continue,
                    };
                    let type_node = match child.child_by_field_name("type") {
                        Some(t) => t,
                        None => continue,
                    };
                    (node_text(&name_node, source), type_node)
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

            // Untyped default: `def foo(x=5)` — emit as Variable (no TypeRef).
            "default_parameter" => {
                let name_node = match child.child_by_field_name("name") {
                    Some(n) => n,
                    None => continue,
                };
                let name = node_text(&name_node, source);
                if name.is_empty() || name == "self" || name == "cls" {
                    continue;
                }
                symbols.push(ExtractedSymbol {
                    name: name.clone(),
                    qualified_name: qualify(&name, func_qualified_name),
                    kind: SymbolKind::Variable,
                    visibility: None,
                    start_line: child.start_position().row as u32,
                    end_line: child.end_position().row as u32,
                    start_col: child.start_position().column as u32,
                    end_col: child.end_position().column as u32,
                    signature: None,
                    doc_comment: None,
                    scope_path: Some(func_qualified_name.to_string()),
                    parent_index,
                });
            }

            // `*args` — list splat parameter.
            "list_splat_pattern" => {
                // The identifier is the child of the splat node.
                let mut cc = child.walk();
                for c in child.children(&mut cc) {
                    if c.kind() == "identifier" {
                        let name = node_text(&c, source);
                        if !name.is_empty() && name != "self" && name != "cls" {
                            symbols.push(ExtractedSymbol {
                                name: name.clone(),
                                qualified_name: qualify(&name, func_qualified_name),
                                kind: SymbolKind::Variable,
                                visibility: None,
                                start_line: c.start_position().row as u32,
                                end_line: c.end_position().row as u32,
                                start_col: c.start_position().column as u32,
                                end_col: c.end_position().column as u32,
                                signature: Some(format!("*{name}")),
                                doc_comment: None,
                                scope_path: Some(func_qualified_name.to_string()),
                                parent_index,
                            });
                        }
                        break;
                    }
                }
            }

            // `**kwargs` — dictionary splat parameter.
            "dictionary_splat_pattern" => {
                let mut cc = child.walk();
                for c in child.children(&mut cc) {
                    if c.kind() == "identifier" {
                        let name = node_text(&c, source);
                        if !name.is_empty() && name != "self" && name != "cls" {
                            symbols.push(ExtractedSymbol {
                                name: name.clone(),
                                qualified_name: qualify(&name, func_qualified_name),
                                kind: SymbolKind::Variable,
                                visibility: None,
                                start_line: c.start_position().row as u32,
                                end_line: c.end_position().row as u32,
                                start_col: c.start_position().column as u32,
                                end_col: c.end_position().column as u32,
                                signature: Some(format!("**{name}")),
                                doc_comment: None,
                                scope_path: Some(func_qualified_name.to_string()),
                                parent_index,
                            });
                        }
                        break;
                    }
                }
            }

            _ => {}
        }
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
        | "list_pattern" | "group_pattern" | "complex_pattern" => {
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

        // `*rest` inside sequence/list patterns — Variable for the binding.
        "splat_pattern" => {
            // The identifier child is the binding name.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = node_text(&child, source);
                    if !name.is_empty() && name != "_" {
                        push_variable_symbol(
                            node,
                            &child,
                            &name,
                            SymbolKind::Variable,
                            symbols,
                            parent_index,
                            qualified_prefix,
                        );
                    }
                }
            }
        }

        // `{key: value_binding}` — recurse into key/value patterns.
        "dict_pattern" => {
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

        // `key: pattern` inside dict_pattern — extract the value pattern binding.
        "key_value_pattern" => {
            // Second named child is the value pattern (the binding).
            if let Some(value_pat) = node.named_child(1) {
                extract_pattern_refs(
                    &value_pat,
                    source,
                    symbols,
                    refs,
                    parent_index,
                    qualified_prefix,
                    enclosing_symbol_index,
                );
            }
        }

        // `identifier` inside a pattern context — treat as a capture variable binding.
        // This handles dotted_name children and direct identifier bindings in dict patterns.
        "identifier" => {
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

        // `dotted_name` wraps identifiers (e.g. `module.ClassName`) in class patterns.
        // Treat a bare dotted_name (single identifier) as a capture, multi-part as TypeRef.
        "dotted_name" => {
            let mut cursor = node.walk();
            let children: Vec<_> = node
                .named_children(&mut cursor)
                .filter(|c| c.kind() == "identifier")
                .collect();
            if children.len() == 1 {
                // Single identifier in dotted_name — capture variable.
                let name = node_text(&children[0], source);
                if !name.is_empty() && name != "_" {
                    push_variable_symbol(
                        node,
                        &children[0],
                        &name,
                        SymbolKind::Variable,
                        symbols,
                        parent_index,
                        qualified_prefix,
                    );
                }
            }
            // Multi-part dotted names are class references; no variable binding here.
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
// For / async-for statements
// =============================================================================

/// Extract loop variables and recurse into the body of a `for_statement` or
/// `async_for_statement`.
///
/// Tree-sitter shape:
/// ```text
/// for_statement / async_for_statement
///   left:  identifier | tuple_pattern | pattern_list
///   right: <iterable expression>
///   body:  block
/// ```
fn extract_for_statement(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
) {
    // Extract loop variable(s).
    if let Some(left) = node.child_by_field_name("left") {
        extract_for_in_vars(&left, source, symbols, parent_index, qualified_prefix, node);
    }
    // Extract calls from the iterable expression.
    if let Some(right) = node.child_by_field_name("right") {
        extract_calls_from_body(&right, source, enclosing_symbol_index, refs);
    }
    // Recurse into the body block.
    if let Some(body) = node.child_by_field_name("body") {
        extract_body_symbols(
            &body,
            source,
            symbols,
            refs,
            parent_index,
            qualified_prefix,
            enclosing_symbol_index,
        );
    }
}

// =============================================================================
// Except clause (try/except)
// =============================================================================

/// Extract TypeRef for the exception type(s) and a Variable for the `as` binding
/// from `except ValueError as e:` or `except (TypeError, ValueError) as e:`.
///
/// Also handles Python 3.11+ `except_group_clause` (`except* ValueError as eg:`).
fn extract_except_clause(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
) {
    // tree-sitter-python 0.25 actual shape:
    //
    //   except_clause
    //     "except"
    //     as_pattern              ← exception type + optional `as var` wrapped together
    //       <type>                ← identifier | tuple
    //       "as"
    //       as_pattern_target
    //         identifier          ← binding variable
    //     block
    //
    // OR without binding:
    //   except_clause
    //     "except"
    //     identifier              ← bare exception type (no `as`)
    //     block
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Most common form: `except ValueError as e:` or `except (T1, T2) as e:`.
            "as_pattern" => {
                // First named child is the exception type (identifier or tuple).
                if let Some(type_node) = child.named_child(0) {
                    extract_except_type_refs(&type_node, source, refs, enclosing_symbol_index);
                }
                // The alias: last named child is the as_pattern_target.
                let n = child.named_child_count();
                if let Some(target) = child.named_child(n.saturating_sub(1)) {
                    let ident = if target.kind() == "as_pattern_target" {
                        target.named_child(0).unwrap_or(target)
                    } else {
                        target
                    };
                    if ident.kind() == "identifier" {
                        let name = node_text(&ident, source);
                        if !name.is_empty() {
                            push_variable_symbol(
                                node,
                                &ident,
                                &name,
                                SymbolKind::Variable,
                                symbols,
                                parent_index,
                                qualified_prefix,
                            );
                        }
                    }
                }
            }
            // Bare exception type without binding: `except ValueError:`.
            "identifier" => {
                let name = node_text(&child, source);
                // Skip the `except` keyword itself (though it's usually anonymous).
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: enclosing_symbol_index,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
            // Recurse into the body block.
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

/// Extract TypeRef edges from an exception type expression inside `except`.
fn extract_except_type_refs(
    node: &Node,
    source: &str,
    refs: &mut Vec<ExtractedRef>,
    enclosing_symbol_index: usize,
) {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source);
            if !name.is_empty() {
                refs.push(ExtractedRef {
                    source_symbol_index: enclosing_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
        }
        "attribute" => {
            if let Some(attr) = node.child_by_field_name("attribute") {
                let name = node_text(&attr, source);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: enclosing_symbol_index,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: attr.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
        }
        // `(TypeError, ValueError)` — tuple of exception types.
        "tuple" => {
            let mut cursor = node.walk();
            for item in node.children(&mut cursor) {
                extract_except_type_refs(&item, source, refs, enclosing_symbol_index);
            }
        }
        _ => {}
    }
}

// =============================================================================
// Raise statement
// =============================================================================

/// Extract calls and TypeRef from a `raise_statement`.
///
/// `raise ValueError("msg")` → Calls edge to `ValueError` constructor and
/// TypeRef edge to `ValueError`.
fn extract_raise_statement(
    node: &Node,
    source: &str,
    enclosing_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Generic call extraction covers `raise Foo(...)` → Calls edge.
    extract_calls_from_body(node, source, enclosing_symbol_index, refs);

    // Additionally emit a TypeRef for the exception class.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `raise SomeError` or `raise SomeError(...)`.
            "identifier" => {
                let name = node_text(&child, source);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: enclosing_symbol_index,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
            "call" => {
                if let Some(func) = child.child_by_field_name("function") {
                    match func.kind() {
                        "identifier" => {
                            let name = node_text(&func, source);
                            if !name.is_empty() {
                                refs.push(ExtractedRef {
                                    source_symbol_index: enclosing_symbol_index,
                                    target_name: name,
                                    kind: EdgeKind::TypeRef,
                                    line: func.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                });
                            }
                        }
                        "attribute" => {
                            if let Some(attr) = func.child_by_field_name("attribute") {
                                let name = node_text(&attr, source);
                                if !name.is_empty() {
                                    refs.push(ExtractedRef {
                                        source_symbol_index: enclosing_symbol_index,
                                        target_name: name,
                                        kind: EdgeKind::TypeRef,
                                        line: attr.start_position().row as u32,
                                        module: None,
                                        chain: None,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

// =============================================================================
// Augmented assignment
// =============================================================================

/// Extract member access from `x += 1` or `self.count += 1`.
fn extract_augmented_assignment(
    node: &Node,
    source: &str,
    enclosing_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Recurse for any calls in the right-hand side.
    extract_calls_from_body(node, source, enclosing_symbol_index, refs);

    // Emit a Calls edge if the left side is an attribute access (member access).
    if let Some(left) = node.child_by_field_name("left") {
        if left.kind() == "attribute" {
            if let Some(chain) = build_chain(&left, source) {
                let target = chain.segments.last().map(|s| s.name.clone()).unwrap_or_default();
                if !target.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index: enclosing_symbol_index,
                        target_name: target,
                        kind: EdgeKind::Calls,
                        line: left.start_position().row as u32,
                        module: None,
                        chain: Some(chain),
                    });
                }
            }
        }
    }
}

// =============================================================================
// Type alias statement (Python 3.12+)
// =============================================================================

/// Public wrapper for top-level `type_alias_statement` nodes (called from mod.rs).
pub(super) fn extract_type_alias_top_level(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
) {
    extract_type_alias(node, source, symbols, refs, parent_index, qualified_prefix, enclosing_symbol_index);
}

/// Extract `type Point = tuple[int, int]` as a TypeAlias symbol.
fn extract_type_alias(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
) {
    // tree-sitter-python uses "left" for the alias name and "right" for the
    // aliased type in `type_alias_statement` (field names differ from "name"/"value").
    let name_node = match node.child_by_field_name("left") {
        Some(n) => n,
        None => return,
    };
    // The "left" child is a "type" wrapper node; the actual identifier is inside it.
    let name = if name_node.kind() == "type" {
        name_node
            .named_child(0)
            .map(|c| node_text(&c, source))
            .unwrap_or_else(|| node_text(&name_node, source))
    } else {
        node_text(&name_node, source)
    };
    let qualified_name = qualify(&name, qualified_prefix);

    let idx = symbols.len();
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::TypeAlias,
        visibility: detect_python_visibility(&name),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("type {name} = ...")),
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });

    // Extract TypeRef edges from the aliased type expression.
    // The "right" field holds the aliased type (also wrapped in a "type" node).
    if let Some(value) = node.child_by_field_name("right") {
        extract_type_refs_from_annotation(&value, source, idx, refs);
    }

    let _ = enclosing_symbol_index; // not used here but kept for API consistency
}

/// Walk a type annotation node and emit TypeRef edges for all identifiers found.
fn extract_type_refs_from_annotation(
    node: &Node,
    source: &str,
    symbol_idx: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source);
            if !name.is_empty() && name != "None" {
                refs.push(ExtractedRef {
                    source_symbol_index: symbol_idx,
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
                    extract_type_refs_from_annotation(&child, source, symbol_idx, refs);
                }
            }
        }
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
