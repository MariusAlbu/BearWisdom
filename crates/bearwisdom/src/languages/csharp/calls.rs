// =============================================================================
// csharp/calls.rs  —  Call, route, and member-chain extraction
// =============================================================================

use super::helpers::node_text;
use super::types::simple_type_name;
use crate::parser::scope_tree::{self, ScopeTree};
use crate::types::{
    ChainSegment, EdgeKind, ExtractedRef, ExtractedRoute, ExtractedSymbol, MemberChain,
    SegmentKind, SymbolKind,
};
use std::collections::HashMap;
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Call extraction
// ---------------------------------------------------------------------------

pub(super) fn extract_calls_from_body(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "invocation_expression" => {
                if let Some(callee) = child.child_by_field_name("function") {
                    let chain = build_chain(callee, src);
                    let name = chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| callee_name(callee, src));
                    crate::languages::emit_chain_type_ref(&chain, source_symbol_index, &callee, refs);
                    if !name.is_empty() && !is_csharp_keyword(&name) {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Calls,
                            line: callee.start_position().row as u32,
                            module: None,
                            chain,
                        });
                    }
                }
                // Recurse into arguments and method chain — calls may be nested.
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            "object_creation_expression" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    let name = simple_type_name(type_node, src);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Instantiates,
                            line: type_node.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                    // Also emit TypeRefs for type arguments: `new Dictionary<string, Foo>()`
                    super::types::extract_type_refs_from_type_node(type_node, src, source_symbol_index, refs);
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `generic_name` in expression position (e.g. method type arguments `Method<Foo>()`,
            // type constraint expressions, etc.) — emit TypeRef for both the name and its args.
            "generic_name" => {
                super::types::extract_type_refs_from_type_node(child, src, source_symbol_index, refs);
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `type_argument_list` in expression position — emit TypeRef for each argument.
            "type_argument_list" => {
                let mut cursor2 = child.walk();
                for arg in child.children(&mut cursor2) {
                    super::types::extract_type_refs_from_type_node(arg, src, source_symbol_index, refs);
                }
            }
            // `user is Admin admin` / `user is Admin` — is_expression or
            // is_pattern_expression (tree-sitter-c-sharp uses both node kinds
            // depending on whether a pattern variable is present).
            "is_expression" | "is_pattern_expression" => {
                extract_is_expression_refs(&child, src, source_symbol_index, refs);
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `user switch { Admin a => a.Level, _ => 0 }` — switch_expression
            // with declaration_pattern or type_pattern arms.
            "switch_expression" => {
                extract_switch_expression_type_refs(&child, src, source_symbol_index, refs);
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `(Admin)user` — cast expression; emit TypeRef for the cast type.
            "cast_expression" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    extract_type_ref_from_cast_type(type_node, src, source_symbol_index, refs);
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `value as Admin` — as_expression; emit TypeRef for the target type.
            // In tree-sitter-c-sharp the type is the `right` field (left=value, right=type).
            "as_expression" => {
                if let Some(type_node) = child.child_by_field_name("right") {
                    extract_type_ref_from_cast_type(type_node, src, source_symbol_index, refs);
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `typeof(Admin)` — emit TypeRef for the argument type.
            "typeof_expression" => {
                // tree-sitter-c-sharp: typeof_expression has a single type child
                // (not a named field in all grammar versions — scan children).
                let mut cursor2 = child.walk();
                for c in child.children(&mut cursor2) {
                    if !matches!(c.kind(), "typeof" | "(" | ")") {
                        extract_type_ref_from_cast_type(c, src, source_symbol_index, refs);
                        break;
                    }
                }
            }
            // `nameof(Symbol)` — emit a Calls-like ref for the named symbol.
            "nameof_expression" => {
                let mut cursor2 = child.walk();
                for c in child.children(&mut cursor2) {
                    if !matches!(c.kind(), "nameof" | "(" | ")") {
                        let name = match c.kind() {
                            "identifier" => node_text(c, src),
                            "member_access_expression" => c
                                .child_by_field_name("name")
                                .map(|n| node_text(n, src))
                                .unwrap_or_else(|| {
                                    let t = node_text(c, src);
                                    t.rsplit('.').next().unwrap_or(&t).to_string()
                                }),
                            _ => {
                                let t = node_text(c, src);
                                t.rsplit('.').next().unwrap_or(&t).to_string()
                            }
                        };
                        if !name.is_empty() && !is_csharp_keyword(&name) {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::TypeRef,
                                line: c.start_position().row as u32,
                                module: None,
                                chain: None,
                            });
                        }
                        break;
                    }
                }
            }
            // `foreach (TypeName item in collection)` — TypeRef for explicit type,
            // plus recurse into the body.
            // Note: tree-sitter-c-sharp uses "foreach_statement" (not "for_each_statement").
            "foreach_statement" => {
                if let Some(type_node) = child.child_by_field_name("type") {
                    extract_type_ref_from_cast_type(type_node, src, source_symbol_index, refs);
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `catch (ExceptionType e)` — TypeRef for the exception type.
            // tree-sitter-c-sharp: catch_clause has an unnamed catch_declaration child
            // (it is not a named field).  Scan children for catch_declaration.
            "catch_clause" => {
                let mut catch_cursor = child.walk();
                for catch_child in child.children(&mut catch_cursor) {
                    if catch_child.kind() == "catch_declaration" {
                        if let Some(type_node) = catch_child.child_by_field_name("type") {
                            extract_type_ref_from_cast_type(type_node, src, source_symbol_index, refs);
                        }
                        break;
                    }
                }
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `using (var x = new Resource())` — recurse; TypeRef extracted from new expr.
            "using_statement" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // Local function statement inside a method body — emit Function symbol
            // and extract calls from its body.
            "local_function_statement" => {
                // Calls within the local function are attributed to the enclosing method.
                if let Some(body) = child.child_by_field_name("body") {
                    extract_calls_from_body(&body, src, source_symbol_index, refs);
                }
            }
            // `x => x.Name` — lambda expression body (single expression form).
            // Also handle `(x, y) => Compute(x, y)` — lambda body may contain calls.
            "lambda_expression" => {
                // Recurse into lambda body to find nested calls.
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `condition ? trueExpr : falseExpr` — ternary expression.
            // Both branches may contain method calls.
            "conditional_expression" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            // `obj?.Property?.Method()` — null-conditional chain.
            // Recurse to find all calls in the chain.
            "null_conditional_member_access_expression" | "null_conditional_invocation_expression" => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
            _ => {
                extract_calls_from_body(&child, src, source_symbol_index, refs);
            }
        }
    }
}

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
    });
}

// ---------------------------------------------------------------------------
// Type narrowing — is expressions and switch expressions
// ---------------------------------------------------------------------------

/// Emit TypeRef edges from `user is Admin admin` or `user is Admin`.
///
/// Tree-sitter-c-sharp structures:
///
/// `is_expression` (no pattern variable):
/// ```text
/// is_expression
///   identifier "user"
///   identifier "Admin"
/// ```
///
/// `is_pattern_expression` (with declaration pattern):
/// ```text
/// is_pattern_expression
///   identifier "user"
///   declaration_pattern
///     identifier "Admin"
///     identifier "admin"
/// ```
fn extract_is_expression_refs(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `is_pattern_expression` with typed pattern: `user is Admin admin`
            "declaration_pattern" | "type_pattern" => {
                emit_pattern_type_ref(&child, src, source_symbol_index, refs);
                return;
            }
            // `is_pattern_expression` with constant pattern: `user is Admin`
            // tree-sitter-c-sharp uses constant_pattern for bare identifier checks.
            // An identifier inside constant_pattern in an `is` context is a type name.
            "constant_pattern" => {
                if let Some(inner) = child.named_child(0) {
                    let type_name = match inner.kind() {
                        "identifier" => node_text(inner, src),
                        "generic_name" => inner
                            .child_by_field_name("name")
                            .map(|n| node_text(n, src))
                            .unwrap_or_else(|| node_text(inner, src)),
                        "qualified_name" => {
                            let full = node_text(inner, src);
                            full.rsplit('.').next().unwrap_or(&full).to_string()
                        }
                        _ => String::new(),
                    };
                    if !type_name.is_empty() && !is_csharp_keyword(&type_name) {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: type_name,
                            kind: EdgeKind::TypeRef,
                            line: inner.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
                return;
            }
            "discard_pattern" => {
                // `_` — no type information.
                return;
            }
            // `x is not Admin` — negated_pattern wraps a sub-pattern
            // (grammar node is `negated_pattern`, not `not_pattern`).
            "negated_pattern" | "not_pattern" => {
                // Recurse into the single sub-pattern.
                let mut cp = child.walk();
                for sub in child.children(&mut cp) {
                    extract_pattern_type_refs_recursive(&sub, src, source_symbol_index, refs);
                }
                return;
            }
            // `x is Admin or User`, `x is > 0 and < 10` — composite patterns.
            "or_pattern" | "and_pattern" | "binary_pattern" | "parenthesized_pattern" => {
                // Recurse into each sub-pattern.
                let mut cp = child.walk();
                for sub in child.children(&mut cp) {
                    extract_pattern_type_refs_recursive(&sub, src, source_symbol_index, refs);
                }
                return;
            }
            "var_pattern" | "relational_pattern" | "list_pattern"
            | "slice_pattern" | "recursive_pattern" => {
                // var/relational/list patterns carry no user type reference.
                return;
            }
            _ => {}
        }
    }

    // Fallback for `is_expression` (older grammar variant): the type follows `is`.
    let mut after_is = false;
    let mut cursor2 = node.walk();
    for child in node.children(&mut cursor2) {
        if child.kind() == "is" {
            after_is = true;
            continue;
        }
        if !after_is {
            continue;
        }
        let type_name = match child.kind() {
            "identifier" => node_text(child, src),
            "generic_name" => {
                let found = child.child_by_field_name("name").or_else(|| {
                    let mut c = child.walk();
                    let kids: Vec<_> = child.children(&mut c).collect();
                    kids.into_iter().find(|cc| cc.kind() == "identifier")
                });
                found.map(|n| node_text(n, src)).unwrap_or_default()
            }
            "qualified_name" => {
                let full = node_text(child, src);
                full.rsplit('.').next().unwrap_or(&full).to_string()
            }
            _ => String::new(),
        };
        if !type_name.is_empty() && !is_csharp_keyword(&type_name) {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: type_name,
                kind: EdgeKind::TypeRef,
                line: child.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
        break;
    }
}

/// Recursively extract TypeRefs from any pattern node.
///
/// Handles composite patterns (`or_pattern`, `and_pattern`, `negated_pattern`,
/// `parenthesized_pattern`) by recursing into children, and terminal patterns
/// (`declaration_pattern`, `type_pattern`, `constant_pattern` with identifier)
/// by emitting a TypeRef.
fn extract_pattern_type_refs_recursive(
    pattern: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match pattern.kind() {
        "declaration_pattern" | "type_pattern" => {
            emit_pattern_type_ref(pattern, src, source_symbol_index, refs);
        }
        "constant_pattern" => {
            if let Some(inner) = pattern.named_child(0) {
                let type_name = match inner.kind() {
                    "identifier" => node_text(inner, src),
                    "generic_name" => inner
                        .child_by_field_name("name")
                        .map(|n| node_text(n, src))
                        .unwrap_or_else(|| node_text(inner, src)),
                    "qualified_name" => {
                        let full = node_text(inner, src);
                        full.rsplit('.').next().unwrap_or(&full).to_string()
                    }
                    _ => String::new(),
                };
                if !type_name.is_empty() && !is_csharp_keyword(&type_name) {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: type_name,
                        kind: EdgeKind::TypeRef,
                        line: inner.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
        }
        "or_pattern" | "and_pattern" | "binary_pattern"
        | "negated_pattern" | "not_pattern"
        | "parenthesized_pattern" => {
            let mut cp = pattern.walk();
            for sub in pattern.children(&mut cp) {
                extract_pattern_type_refs_recursive(&sub, src, source_symbol_index, refs);
            }
        }
        // relational, discard, var, list, slice, recursive — no user type refs
        _ => {}
    }
}

/// Emit a TypeRef for the type in a `declaration_pattern` or `type_pattern`.
///
/// ```text
/// declaration_pattern
///   identifier "Admin"   ← type (field: "type")
///   identifier "admin"   ← designator
/// ```
fn emit_pattern_type_ref(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let type_node = node.child_by_field_name("type").or_else(|| {
        let mut cursor = node.walk();
        let kids: Vec<_> = node.children(&mut cursor).collect();
        kids.into_iter()
            .find(|c| matches!(c.kind(), "identifier" | "generic_name" | "qualified_name"))
    });

    if let Some(type_node) = type_node {
        let type_name = match type_node.kind() {
            "identifier" => node_text(type_node, src),
            "generic_name" => type_node
                .child_by_field_name("name")
                .map(|n| node_text(n, src))
                .unwrap_or_else(|| node_text(type_node, src)),
            "qualified_name" => {
                let full = node_text(type_node, src);
                full.rsplit('.').next().unwrap_or(&full).to_string()
            }
            _ => node_text(type_node, src),
        };
        if !type_name.is_empty() && !is_csharp_keyword(&type_name) {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: type_name,
                kind: EdgeKind::TypeRef,
                line: type_node.start_position().row as u32,
                module: None,
                chain: None,
            });
        }
    }
}

/// Emit TypeRefs for each type-bearing arm of a `switch_expression`.
///
/// ```csharp
/// var result = user switch {
///     Admin a => a.Level,
///     User u  => 0,
///     _       => -1,
/// };
/// ```
/// Tree-sitter-c-sharp: `switch_expression` → `switch_expression_arm` children,
/// each with a `pattern` field that may be a `declaration_pattern`.
fn extract_switch_expression_type_refs(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for arm in node.children(&mut cursor) {
        if arm.kind() != "switch_expression_arm" {
            continue;
        }
        // The pattern is either the `pattern` field or the first named child.
        let pattern = arm
            .child_by_field_name("pattern")
            .or_else(|| arm.named_child(0));
        if let Some(pattern) = pattern {
            // Use the recursive helper so composite patterns (or/and/not) are
            // walked fully, not just top-level declaration/type patterns.
            extract_pattern_type_refs_recursive(&pattern, src, source_symbol_index, refs);
        }
    }
}

/// Emit a TypeRef for a cast/typeof/as target type node.
/// Delegates to the shared type-node walker but skips builtins.
fn extract_type_ref_from_cast_type(
    type_node: Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    use super::types::extract_type_refs_from_type_node;
    extract_type_refs_from_type_node(type_node, src, source_symbol_index, refs);
}

/// C# keywords/operators that look like method calls but aren't.
pub(super) fn is_csharp_keyword(name: &str) -> bool {
    matches!(
        name,
        "nameof" | "typeof" | "sizeof" | "default" | "checked" | "unchecked"
        | "stackalloc" | "await" | "throw" | "yield" | "var" | "is" | "as"
        | "new" | "this" | "base" | "null" | "true" | "false" | "value"
    )
}

fn callee_name(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "identifier" => node_text(node, src),
        "member_access_expression" => node
            .child_by_field_name("name")
            .map(|n| node_text(n, src))
            .unwrap_or_else(|| {
                let t = node_text(node, src);
                t.rsplit('.').next().unwrap_or(&t).to_string()
            }),
        "generic_name" => {
            // Generic method call like `GetService<T>()` — extract just the name.
            let children: Vec<Node> = {
                let mut cursor = node.walk();
                node.children(&mut cursor).collect()
            };
            children
                .iter()
                .find(|c| c.kind() == "identifier")
                .map(|n| node_text(*n, src))
                .unwrap_or_default()
        }
        _ => {
            let t = node_text(node, src);
            t.rsplit('.').next().unwrap_or(&t).to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// MemberChain building
// ---------------------------------------------------------------------------

/// Build a structured member access chain from tree-sitter AST nodes.
///
/// Recursively walks nested `member_access_expression` nodes to produce
/// a `Vec<ChainSegment>` from root to leaf.
///
/// `this.repo.FindOne()` tree structure:
/// ```text
/// invocation_expression
///   function: member_access_expression
///     expression: member_access_expression
///       expression: this_expression "this"
///       name: identifier "repo"
///     name: identifier "FindOne"
/// ```
/// produces: `[this, repo, FindOne]`
pub(super) fn build_chain(node: Node, src: &[u8]) -> Option<MemberChain> {
    let mut segments = Vec::new();
    build_chain_inner(node, src, &mut segments)?;
    if segments.is_empty() {
        return None;
    }
    Some(MemberChain { segments })
}

fn build_chain_inner(node: Node, src: &[u8], segments: &mut Vec<ChainSegment>) -> Option<()> {
    match node.kind() {
        "this_expression" => {
            segments.push(ChainSegment {
                name: "this".to_string(),
                node_kind: "this_expression".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "base_expression" => {
            segments.push(ChainSegment {
                name: "base".to_string(),
                node_kind: "base_expression".to_string(),
                kind: SegmentKind::SelfRef,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "identifier" => {
            segments.push(ChainSegment {
                name: node_text(node, src),
                node_kind: "identifier".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "generic_name" => {
            // `GetService<T>` — strip the generic args, keep just the identifier.
            let name = {
                let mut cursor = node.walk();
                let children: Vec<Node> = node.children(&mut cursor).collect();
                drop(cursor);
                children
                    .iter()
                    .find(|c| c.kind() == "identifier")
                    .map(|c| node_text(*c, src))
                    .unwrap_or_else(|| node_text(node, src))
            };
            segments.push(ChainSegment {
                name,
                node_kind: "generic_name".to_string(),
                kind: SegmentKind::Identifier,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "member_access_expression" => {
            let expr = node.child_by_field_name("expression")?;
            let name_node = node.child_by_field_name("name")?;

            // Recurse into the expression (receiver) to build the prefix chain.
            build_chain_inner(expr, src, segments)?;

            // The name may be a generic_name (e.g., `Foo<T>`) — extract identifier.
            let name = if name_node.kind() == "generic_name" {
                let mut cursor = name_node.walk();
                let children: Vec<Node> = name_node.children(&mut cursor).collect();
                drop(cursor);
                children
                    .iter()
                    .find(|c| c.kind() == "identifier")
                    .map(|c| node_text(*c, src))
                    .unwrap_or_else(|| node_text(name_node, src))
            } else {
                node_text(name_node, src)
            };

            segments.push(ChainSegment {
                name,
                node_kind: name_node.kind().to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: false,
            });
            Some(())
        }

        "conditional_access_expression" => {
            // C# `?.` operator: `foo?.Bar()`
            let expr = node.child_by_field_name("expression")?;
            let binding = node.child_by_field_name("binding")?;

            build_chain_inner(expr, src, segments)?;

            // The binding is a `member_binding_expression` with a `name` field.
            let name_node = binding.child_by_field_name("name").unwrap_or(binding);
            segments.push(ChainSegment {
                name: node_text(name_node, src),
                node_kind: binding.kind().to_string(),
                kind: SegmentKind::Property,
                declared_type: None,
                type_args: vec![],
                optional_chaining: true,
            });
            Some(())
        }

        "invocation_expression" => {
            // Nested call in a chain: `a.B().C()` — the expression is an invocation.
            // Walk into the function child to continue the chain.
            let func = node.child_by_field_name("function")?;
            build_chain_inner(func, src, segments)
        }

        // Unknown node — can't build a chain.
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// HTTP Route extraction
// ---------------------------------------------------------------------------

/// Extract the class-level `[Route("...")]` attribute value for ASP.NET controllers.
///
/// Example: `[Route("api/categories")]` → `Some("api/categories")`
pub(super) fn extract_class_route_prefix(class_node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = class_node.walk();
    for child in class_node.children(&mut cursor) {
        if child.kind() == "attribute_list" {
            let mut al_cursor = child.walk();
            for attr in child.children(&mut al_cursor) {
                if attr.kind() == "attribute" {
                    if let Some(name_node) = attr.child_by_field_name("name") {
                        let name = node_text(name_node, src);
                        if name == "Route" {
                            return attr_route_template(&attr, src);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Attribute-based route extraction with optional class-level prefix.
pub(super) fn extract_attribute_routes_with_prefix(
    node: &Node,
    src: &[u8],
    handler_symbol_index: usize,
    routes: &mut Vec<ExtractedRoute>,
    class_prefix: Option<&str>,
) {
    let mut outer = node.walk();
    for child in node.children(&mut outer) {
        if child.kind() == "attribute_list" {
            let mut al_cursor = child.walk();
            for attr in child.children(&mut al_cursor) {
                if attr.kind() == "attribute" {
                    if let Some(name_node) = attr.child_by_field_name("name") {
                        let attr_name = node_text(name_node, src);
                        if let Some(method) = http_method_from_attribute(&attr_name) {
                            let method_template = attr_route_template(&attr, src)
                                .unwrap_or_else(|| String::from(""));
                            // Combine class prefix with method template.
                            let template = match class_prefix {
                                Some(prefix) if !prefix.is_empty() => {
                                    let p = prefix.trim_matches('/');
                                    let m = method_template.trim_matches('/');
                                    if m.is_empty() {
                                        format!("/{p}")
                                    } else {
                                        format!("/{p}/{m}")
                                    }
                                }
                                _ => {
                                    if method_template.is_empty() {
                                        "/".to_string()
                                    } else {
                                        method_template
                                    }
                                }
                            };
                            routes.push(ExtractedRoute {
                                handler_symbol_index,
                                http_method: method.to_string(),
                                template,
                            });
                        }
                    }
                }
            }
        }
    }
}

pub(super) fn http_method_from_attribute(name: &str) -> Option<&'static str> {
    // Strip generic suffix if present: `HttpGet<T>` → `HttpGet`
    let base = name.split('<').next().unwrap_or(name);
    match base {
        "HttpGet" | "MapGet" => Some("GET"),
        "HttpPost" | "MapPost" => Some("POST"),
        "HttpPut" | "MapPut" => Some("PUT"),
        "HttpDelete" | "MapDelete" => Some("DELETE"),
        "HttpPatch" | "MapPatch" => Some("PATCH"),
        "Route" => Some("ANY"),
        _ => None,
    }
}

pub(super) fn attr_route_template(attr_node: &Node, src: &[u8]) -> Option<String> {
    use super::helpers::find_child_kind;
    // In tree-sitter-c-sharp the attribute argument list is a child NODE of kind
    // `attribute_argument_list` — it is NOT a named field, so child_by_field_name
    // will always return None.  We must find it by kind.
    //
    // Structure:
    //   attribute
    //     identifier              ← name (this IS a named field)
    //     attribute_argument_list ← kind (NOT a named field)
    //       (
    //       attribute_argument
    //         string_literal
    //           string_literal_content  ← raw text, no quotes
    //       )
    let arg_list = find_child_kind(attr_node, "attribute_argument_list")?;
    let mut cursor = arg_list.walk();
    for arg in arg_list.children(&mut cursor) {
        if arg.kind() == "attribute_argument" {
            let mut ac = arg.walk();
            for child in arg.children(&mut ac) {
                match child.kind() {
                    "string_literal" => {
                        // Prefer string_literal_content (the text without surrounding quotes).
                        let children: Vec<Node> = {
                            let mut sc = child.walk();
                            child.children(&mut sc).collect()
                        };
                        if let Some(content) = children.iter().find(|c| c.kind() == "string_literal_content") {
                            return Some(node_text(*content, src));
                        }
                        // Fallback: strip quotes from the whole string_literal text.
                        let raw = node_text(child, src);
                        return Some(raw.trim_matches('"').to_string());
                    }
                    "verbatim_string_literal" => {
                        let raw = node_text(child, src);
                        let stripped = raw.trim_start_matches('@').trim_matches('"');
                        return Some(stripped.to_string());
                    }
                    "interpolated_string_expression" => {
                        return Some("{dynamic}".to_string());
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

/// Combine a route prefix with a route template.
///
/// Examples:
///   ("api/auth", "login")       → "api/auth/login"
///   ("api/auth", "/")           → "api/auth"
///   ("", "login")               → "login"
///   ("api/catalog", "{id:int}") → "api/catalog/{id:int}"
pub(super) fn combine_route_prefix(prefix: &str, action: &str) -> String {
    let prefix = prefix.trim_matches('/');
    let action = action.trim_matches('/');

    if prefix.is_empty() {
        return if action.is_empty() { "/".to_string() } else { action.to_string() };
    }
    if action.is_empty() {
        return prefix.to_string();
    }
    format!("{prefix}/{action}")
}

/// Minimal-API route registration inside method bodies:
///   `app.MapGet("/api/items", ...)` etc.
///
/// Also resolves `MapGroup` prefixes:
///   `var api = app.MapGroup("api/orders"); api.MapGet("/", handler);`
///   → route template becomes `"api/orders"` instead of `"/"`.
pub(super) fn extract_minimal_api_routes(
    body: &Node,
    src: &[u8],
    handler_symbol_index: usize,
    routes: &mut Vec<ExtractedRoute>,
) {
    let group_prefixes = build_mapgroup_prefixes(body, src);
    extract_minimal_api_routes_inner(body, src, handler_symbol_index, routes, &group_prefixes);
}

/// Build a map of variable names to their accumulated MapGroup prefix.
fn build_mapgroup_prefixes<'a>(body: &Node<'a>, src: &[u8]) -> HashMap<String, String> {
    let mut prefixes: HashMap<String, String> = HashMap::new();
    collect_mapgroup_assignments(body, src, &mut prefixes);
    prefixes
}

/// Recursively walk a block collecting `var X = expr.MapGroup("prefix")` assignments.
fn collect_mapgroup_assignments(node: &Node, src: &[u8], prefixes: &mut HashMap<String, String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "local_declaration_statement"
            || child.kind() == "variable_declaration"
        {
            collect_mapgroup_assignments(&child, src, prefixes);
            continue;
        }

        if child.kind() == "variable_declarator" {
            let var_name = child
                .child_by_field_name("name")
                .map(|n| node_text(n, src));

            // The initializer is a direct child of variable_declarator after `=`.
            let mut found_eq = false;
            let mut init_expr: Option<Node> = None;
            let mut vc = child.walk();
            for vchild in child.children(&mut vc) {
                if vchild.kind() == "=" {
                    found_eq = true;
                } else if found_eq && vchild.kind() == "invocation_expression" {
                    init_expr = Some(vchild);
                    break;
                }
            }

            if let (Some(var_name), Some(init)) = (var_name, init_expr) {
                if let Some(prefix) = resolve_mapgroup_chain(&init, src, prefixes) {
                    prefixes.insert(var_name, prefix);
                }
            }
            continue;
        }

        collect_mapgroup_assignments(&child, src, prefixes);
    }
}

/// Resolve the group prefix from a (possibly chained) expression.
fn resolve_mapgroup_chain(
    node: &Node,
    src: &[u8],
    prefixes: &HashMap<String, String>,
) -> Option<String> {
    if node.kind() != "invocation_expression" {
        return None;
    }

    let func_node = node.child_by_field_name("function")?;

    if func_node.kind() == "member_access_expression" {
        let method_name = node_text(func_node.child_by_field_name("name")?, src);
        let object = func_node.child_by_field_name("expression")?;

        if method_name == "MapGroup" {
            let arg_list = node.child_by_field_name("arguments")?;
            let group_path = first_string_arg(&arg_list, src)?;
            let receiver_prefix = resolve_receiver_prefix(&object, src, prefixes);

            return Some(combine_route_prefix(
                &receiver_prefix.unwrap_or_default(),
                &group_path,
            ));
        }

        // Fluent chain: `.HasApiVersion(...)`, etc. — recurse into the object.
        return resolve_mapgroup_chain(&object, src, prefixes);
    }

    None
}

/// Get the accumulated prefix for a receiver expression.
fn resolve_receiver_prefix(
    object: &Node,
    src: &[u8],
    prefixes: &HashMap<String, String>,
) -> Option<String> {
    match object.kind() {
        "identifier" => {
            let name = node_text(*object, src);
            prefixes.get(&name).cloned()
        }
        "invocation_expression" => resolve_mapgroup_chain(object, src, prefixes),
        _ => None,
    }
}

/// Get the variable name from the receiver of a member_access_expression.
fn get_receiver_name(func_node: &Node, src: &[u8]) -> Option<String> {
    let object = func_node.child_by_field_name("expression")?;
    if object.kind() == "identifier" {
        Some(node_text(object, src))
    } else {
        None
    }
}

/// Inner recursive route extractor with group prefix support.
fn extract_minimal_api_routes_inner(
    body: &Node,
    src: &[u8],
    handler_symbol_index: usize,
    routes: &mut Vec<ExtractedRoute>,
    group_prefixes: &HashMap<String, String>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "invocation_expression" {
            if let Some(func_node) = child.child_by_field_name("function") {
                if func_node.kind() == "member_access_expression" {
                    if let Some(method_name_node) = func_node.child_by_field_name("name") {
                        let method_name = node_text(method_name_node, src);
                        if let Some(http_method) = http_method_from_attribute(&method_name) {
                            if let Some(arg_list) = child.child_by_field_name("arguments") {
                                if let Some(template) = first_string_arg(&arg_list, src) {
                                    let prefix = get_receiver_name(&func_node, src)
                                        .and_then(|name| group_prefixes.get(&name).cloned())
                                        .unwrap_or_default();

                                    let full_template = if prefix.is_empty() {
                                        template
                                    } else {
                                        combine_route_prefix(&prefix, &template)
                                    };

                                    routes.push(ExtractedRoute {
                                        handler_symbol_index,
                                        http_method: http_method.to_string(),
                                        template: full_template,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        extract_minimal_api_routes_inner(&child, src, handler_symbol_index, routes, group_prefixes);
    }
}

pub(super) fn first_string_arg(arg_list: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = arg_list.walk();
    for arg in arg_list.children(&mut cursor) {
        if arg.kind() == "argument" {
            let mut ac = arg.walk();
            for child in arg.children(&mut ac) {
                match child.kind() {
                    "string_literal" => {
                        // Prefer the `string_literal_content` child (no surrounding quotes).
                        let children: Vec<Node> = {
                            let mut sc = child.walk();
                            child.children(&mut sc).collect()
                        };
                        if let Some(content) = children.iter().find(|c| c.kind() == "string_literal_content") {
                            return Some(node_text(*content, src));
                        }
                        let raw = node_text(child, src);
                        return Some(raw.trim_matches('"').to_string());
                    }
                    "verbatim_string_literal" => {
                        let raw = node_text(child, src);
                        return Some(raw.trim_start_matches('@').trim_matches('"').to_string());
                    }
                    _ => {}
                }
            }
        }
    }
    None
}
