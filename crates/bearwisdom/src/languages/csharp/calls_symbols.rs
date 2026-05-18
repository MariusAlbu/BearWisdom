// =============================================================================
// csharp/calls_symbols.rs  —  Local variable symbol extraction from method bodies
// =============================================================================

use super::calls::is_csharp_keyword;
use super::helpers::node_text;
use crate::parser::scope_tree::{self, ScopeTree};
use crate::types::{ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Variable symbol extraction — lambdas, LINQ, pattern bindings
// ---------------------------------------------------------------------------

/// Walk a method/constructor body and extract Variable symbols for:
///
/// 1. **Lambda parameters** — `u` in `Select(u => u.Name)`, or `(x, y)` in
///    `Map((x, y) => Combine(x, y))`.
///
///    Tree-sitter: `lambda_expression` with either an `implicit_parameter`
///    (single bare identifier) or a `parameter_list` child.
///
/// 2. **LINQ range variables** — `u` in `from u in users`.
///
///    Tree-sitter: `query_expression` → `from_clause` → the first identifier
///    child that is NOT the `in` keyword context (i.e. the range variable).
///
/// 3. **Pattern binding variables** — `a` in `user is Admin a` or
///    `Admin a =>` arms of a switch expression.
///
///    Tree-sitter: `declaration_pattern` has two identifier children — the
///    first is the type name and the second is the bound variable name.
///    The TypeRef for the type is already emitted by `extract_calls_from_body`.
///    Here we emit only the Variable symbol for the bound name.
pub(super) fn extract_body_variable_symbols(
    node: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "lambda_expression" => {
                extract_lambda_param_symbols(&child, src, scope_tree, symbols, parent_index);
                // Recurse into the lambda body.
                extract_body_variable_symbols(&child, src, scope_tree, symbols, parent_index);
            }
            "query_expression" => {
                extract_linq_range_variables(&child, src, scope_tree, symbols, parent_index);
                // Recurse in case of nested lambdas/queries inside clauses.
                extract_body_variable_symbols(&child, src, scope_tree, symbols, parent_index);
            }
            "declaration_pattern" => {
                extract_pattern_binding_variable(&child, src, scope_tree, symbols, parent_index);
            }
            // Local function statements inside a method/constructor body — emit
            // a Function symbol then recurse into the body for nested lambdas/LINQ.
            "local_function_statement" => {
                let local_idx = super::symbols::push_local_function_decl(
                    &child, src, scope_tree, symbols, parent_index,
                );
                if let Some(body) = child.child_by_field_name("body") {
                    extract_body_variable_symbols(&body, src, scope_tree, symbols, local_idx.or(parent_index));
                }
            }
            _ => {
                extract_body_variable_symbols(&child, src, scope_tree, symbols, parent_index);
            }
        }
    }
}

/// Extract parameters from a `lambda_expression` as Variable symbols.
///
/// Handles:
/// - Single-param shorthand: `lambda_expression` → `implicit_parameter` (identifier)
/// - Multi-param: `lambda_expression` → `parameter_list` → `parameter` nodes
fn extract_lambda_param_symbols(
    lambda: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let scope = if lambda.start_byte() > 0 {
        scope_tree::find_scope_at(scope_tree, lambda.start_byte())
    } else {
        None
    };

    let mut cursor = lambda.walk();
    for child in lambda.children(&mut cursor) {
        match child.kind() {
            // Single bare parameter: `u => u.Name`
            "implicit_parameter" => {
                let name = node_text(child, src);
                if !name.is_empty() && !is_csharp_keyword(&name) {
                    push_variable_symbol(
                        name,
                        child.start_position().row as u32,
                        child.end_position().row as u32,
                        child.start_position().column as u32,
                        child.end_position().column as u32,
                        scope,
                        scope_tree,
                        symbols,
                        parent_index,
                    );
                }
            }
            // Parenthesised parameters: `(x, y) => ...`
            "parameter_list" => {
                let mut pl_cursor = child.walk();
                for param in child.children(&mut pl_cursor) {
                    if param.kind() == "parameter" {
                        if let Some(name_node) = param.child_by_field_name("name") {
                            let name = node_text(name_node, src);
                            if !name.is_empty() && !is_csharp_keyword(&name) {
                                push_variable_symbol(
                                    name,
                                    param.start_position().row as u32,
                                    param.end_position().row as u32,
                                    param.start_position().column as u32,
                                    param.end_position().column as u32,
                                    scope,
                                    scope_tree,
                                    symbols,
                                    parent_index,
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Extract the range variable from a LINQ `query_expression`.
///
/// `from_clause` structure (tree-sitter-c-sharp):
/// ```text
/// from_clause
///   "from"       ← keyword
///   identifier   ← range variable  (e.g. "u")
///   "in"         ← keyword
///   expression   ← data source     (e.g. "users")
/// ```
fn extract_linq_range_variables(
    query: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let mut cursor = query.walk();
    for clause in query.children(&mut cursor) {
        if clause.kind() == "from_clause" {
            // Scan children: skip "from", take first identifier as range var, skip "in" and rest.
            let mut seen_from = false;
            let mut found_var = false;
            let mut cl_cursor = clause.walk();
            for child in clause.children(&mut cl_cursor) {
                if child.kind() == "from" {
                    seen_from = true;
                    continue;
                }
                if seen_from && !found_var && child.kind() == "identifier" {
                    let name = node_text(child, src);
                    if !name.is_empty() {
                        let scope = if child.start_byte() > 0 {
                            scope_tree::find_scope_at(scope_tree, child.start_byte())
                        } else {
                            None
                        };
                        push_variable_symbol(
                            name,
                            child.start_position().row as u32,
                            child.end_position().row as u32,
                            child.start_position().column as u32,
                            child.end_position().column as u32,
                            scope,
                            scope_tree,
                            symbols,
                            parent_index,
                        );
                        found_var = true;
                    }
                }
            }
        }
    }
}

/// Extract the binding variable from a `declaration_pattern`.
///
/// Handles two forms:
///
/// `Admin a` — two identifiers: type name + bound variable:
/// ```text
/// declaration_pattern
///   identifier "Admin"   ← type
///   identifier "a"       ← designator
/// ```
///
/// `var v` — implicit_type (not an identifier) + bound variable identifier:
/// ```text
/// declaration_pattern
///   implicit_type "var"  ← type (NOT an identifier)
///   identifier "v"       ← designator
/// ```
///
/// The TypeRef for the explicit type is handled by `extract_calls_from_body`.
/// Here we emit only the Variable symbol for the bound name.
fn extract_pattern_binding_variable(
    pattern: &Node,
    src: &[u8],
    scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    // Collect identifier children.
    let mut cursor = pattern.walk();
    let idents: Vec<Node> = pattern
        .children(&mut cursor)
        .filter(|c| c.kind() == "identifier")
        .collect();

    // If there are two identifiers (e.g. `Admin a`), the second is the bound variable.
    // If there is one identifier (e.g. `var v` where the type is implicit_type),
    // that single identifier IS the bound variable.
    let name_node = match idents.len() {
        2 => idents.get(1).copied(),
        1 => {
            // Only emit as a variable if the sibling type node is `implicit_type` (var pattern).
            let has_implicit_type = (0..pattern.child_count())
                .filter_map(|i| pattern.child(i))
                .any(|ch| ch.kind() == "implicit_type");
            if has_implicit_type { idents.first().copied() } else { None }
        }
        _ => None,
    };

    if let Some(name_node) = name_node {
        let name = node_text(name_node, src);
        if !name.is_empty() && !is_csharp_keyword(&name) {
            let scope = if name_node.start_byte() > 0 {
                scope_tree::find_scope_at(scope_tree, name_node.start_byte())
            } else {
                None
            };
            push_variable_symbol(
                name,
                name_node.start_position().row as u32,
                name_node.end_position().row as u32,
                name_node.start_position().column as u32,
                name_node.end_position().column as u32,
                scope,
                scope_tree,
                symbols,
                parent_index,
            );
        }
    }
}

/// Push a single Variable symbol using the given scope for qualification.
#[allow(clippy::too_many_arguments)]
fn push_variable_symbol(
    name: String,
    start_line: u32,
    end_line: u32,
    start_col: u32,
    end_col: u32,
    scope: Option<&scope_tree::ScopeEntry>,
    _scope_tree: &ScopeTree,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) {
    let qualified_name = scope_tree::qualify(&name, scope);
    let scope_path = scope_tree::scope_path(scope);
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Variable,
        visibility: None,
        start_line,
        end_line,
        start_col,
        end_col,
        signature: None,
        doc_comment: None,
        scope_path,
        parent_index,
            byte_offset: 0,
    declared_type: None,
    return_type: None,
    param_types: Vec::new(),
    generic_params: Vec::new(),
});
}
