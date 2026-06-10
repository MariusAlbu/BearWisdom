// =============================================================================
// python/statements.rs — statement-form local binders (with/for/except/match/comprehension/walrus)
// =============================================================================

use super::assignments::push_variable_symbol;
use super::calls::{build_chain, extract_calls_from_body};
use super::helpers::{detect_python_visibility, node_text, qualify, scope_from_prefix};
use super::symbols::extract_body_symbols;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use std::collections::HashMap;
use tree_sitter::Node;

pub(super) fn extract_with_statement(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
    import_map: &HashMap<String, String>,
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
                            import_map,
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
                    import_map,
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
                    import_map,
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
    import_map: &HashMap<String, String>,
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
    let value_node = node
        .child_by_field_name("value")
        .or_else(|| node.named_child(0));

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
        extract_calls_from_body(expr, source, enclosing_symbol_index, refs, import_map);
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
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
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
                                is_import_binding: false,
                                is_reexport: false,
                                source_symbol_index: sym_idx,
                                target_name: target,
                                kind: EdgeKind::TypeRef,
                                line: expr.start_position().row as u32,
                                col: 0,
                                module: None,
                                chain: Some(chain),
                                byte_offset: expr.start_byte() as u32,
                                namespace_segments: Vec::new(),
                                call_args: Vec::new(),
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
    import_map: &HashMap<String, String>,
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
                    extract_calls_from_body(
                        &right,
                        source,
                        enclosing_symbol_index,
                        refs,
                        import_map,
                    );
                }
            }
            _ => {
                extract_calls_from_body(&child, source, enclosing_symbol_index, refs, import_map);
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
                    byte_offset: 0,
                    declared_type: None,
                    return_type: None,
                    param_types: Vec::new(),
                    generic_params: Vec::new(),
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
                            byte_offset: 0,
                            declared_type: None,
                            return_type: None,
                            param_types: Vec::new(),
                            generic_params: Vec::new(),
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

pub(super) fn extract_named_expression(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
    import_map: &HashMap<String, String>,
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
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });

    extract_calls_from_body(
        &value_node,
        source,
        enclosing_symbol_index,
        refs,
        import_map,
    );

    if value_node.kind() == "call" {
        if let Some(func) = value_node.child_by_field_name("function") {
            if let Some(chain) = build_chain(&func, source) {
                let target = chain
                    .segments
                    .last()
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                if !target.is_empty() {
                    refs.push(ExtractedRef {
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index: sym_idx,
                        target_name: target,
                        kind: EdgeKind::TypeRef,
                        line: value_node.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: Some(chain),
                        byte_offset: value_node.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
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
    import_map: &HashMap<String, String>,
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
                            import_map,
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
                    import_map,
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
    import_map: &HashMap<String, String>,
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
                    import_map,
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
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index: enclosing_symbol_index,
                        target_name: class_name,
                        kind: EdgeKind::TypeRef,
                        line: class_node.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: class_node.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
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

        "or_pattern" | "union_pattern" | "sequence_pattern" | "tuple_pattern" | "list_pattern"
        | "group_pattern" | "complex_pattern" => {
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

pub(super) fn extract_for_statement(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
    import_map: &HashMap<String, String>,
) {
    // Extract loop variable(s).
    if let Some(left) = node.child_by_field_name("left") {
        extract_for_in_vars(&left, source, symbols, parent_index, qualified_prefix, node);
    }
    // Extract calls from the iterable expression.
    if let Some(right) = node.child_by_field_name("right") {
        extract_calls_from_body(&right, source, enclosing_symbol_index, refs, import_map);
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
            import_map,
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
pub(super) fn extract_except_clause(
    node: &Node,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
    enclosing_symbol_index: usize,
    import_map: &HashMap<String, String>,
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
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index: enclosing_symbol_index,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
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
                    import_map,
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
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index: enclosing_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    col: 0,
                    module: None,
                    chain: None,
                    byte_offset: node.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                });
            }
        }
        "attribute" => {
            if let Some(attr) = node.child_by_field_name("attribute") {
                let name = node_text(&attr, source);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index: enclosing_symbol_index,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: attr.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: attr.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
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
pub(super) fn extract_raise_statement(
    node: &Node,
    source: &str,
    enclosing_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
    import_map: &HashMap<String, String>,
) {
    // Generic call extraction covers `raise Foo(...)` → Calls edge.
    extract_calls_from_body(node, source, enclosing_symbol_index, refs, import_map);

    // Additionally emit a TypeRef for the exception class.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `raise SomeError` or `raise SomeError(...)`.
            "identifier" => {
                let name = node_text(&child, source);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index: enclosing_symbol_index,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: child.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
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
                                    is_import_binding: false,
                                    is_reexport: false,
                                    source_symbol_index: enclosing_symbol_index,
                                    target_name: name,
                                    kind: EdgeKind::TypeRef,
                                    line: func.start_position().row as u32,
                                    col: 0,
                                    module: None,
                                    chain: None,
                                    byte_offset: func.start_byte() as u32,
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
                                });
                            }
                        }
                        "attribute" => {
                            if let Some(attr) = func.child_by_field_name("attribute") {
                                let name = node_text(&attr, source);
                                if !name.is_empty() {
                                    refs.push(ExtractedRef {
                                        is_import_binding: false,
                                        is_reexport: false,
                                        source_symbol_index: enclosing_symbol_index,
                                        target_name: name,
                                        kind: EdgeKind::TypeRef,
                                        line: attr.start_position().row as u32,
                                        col: 0,
                                        module: None,
                                        chain: None,
                                        byte_offset: attr.start_byte() as u32,
                                        namespace_segments: Vec::new(),
                                        call_args: Vec::new(),
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
