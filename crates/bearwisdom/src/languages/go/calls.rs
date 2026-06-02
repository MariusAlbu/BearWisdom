// =============================================================================
// go/calls.rs  —  Call and reference extraction entry points for Go
//
// Owns the per-body traversal (`extract_body_with_symbols`) and call-argument
// extraction (`extract_call_args`). The reference dispatcher and the per-
// construct emitters live in sibling modules:
//   * `refs.rs`       — `extract_refs_from_body` dispatcher
//   * `call_sites.rs` — call / composite-literal / type-assertion emitters
// =============================================================================

use super::helpers::node_text;
use crate::types::{CallArg, ExtractedRef, ExtractedSymbol};
use tree_sitter::Node;

// Re-export items external callers (symbols.rs, statements.rs, types.rs) reach
// for via `super::calls::*`. Carving the file into siblings is an internal
// reorganization — the public surface visible across the language plugin stays
// the same.
pub(super) use super::call_sites::{
    extract_composite_literal_ref, extract_type_assertion_ref, extract_type_switch_refs,
};
pub(super) use super::chain::build_chain;
pub(super) use super::refs::extract_refs_from_body;
pub(super) use super::type_refs::extract_fn_signature_type_refs;

/// Maximum nesting depth for recursive `CallArg` construction. Arguments
/// deeper than this collapse to `CallArg::Other` rather than recursing further.
const MAX_ARG_DEPTH: u32 = 8;

/// Extract positional arguments from a Go `call_expression`'s
/// `argument_list`. Captures interpreted/raw string literals, identifiers
/// (incl. `&entity` unary-pointer wrappers), integers, floats, and
/// booleans. The address-of unary expression (`&User{}`) is detected and
/// stored as `Ident(name)` to support gorm model-pointer extraction.
/// Variadic spread (`args...`), subscript (`m[k]`), and binary expressions
/// produce the corresponding recursive `CallArg` variants.
pub(super) fn extract_call_args(call_node: &Node, src: &str) -> Vec<CallArg> {
    let Some(args_node) = call_node.child_by_field_name("arguments") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut cursor = args_node.walk();
    for child in args_node.named_children(&mut cursor) {
        out.push(extract_arg(&child, src, 0));
    }
    out
}

/// Convert a single Go argument expression node to a `CallArg`, recursing for
/// composite expression kinds up to `MAX_ARG_DEPTH`.
fn extract_arg(node: &Node, src: &str, depth: u32) -> CallArg {
    if depth >= MAX_ARG_DEPTH {
        return CallArg::Other;
    }
    match node.kind() {
        "interpreted_string_literal" | "raw_string_literal" => {
            let raw = node_text(node, src);
            CallArg::StringLit(strip_go_string(&raw))
        }
        "identifier" | "type_identifier" => CallArg::Ident(node_text(node, src)),
        // `&User{...}` — address-of a composite literal. Extract the
        // type name so gorm `db.First(&user)` style detection can
        // resolve to the model.
        "unary_expression" => {
            if let Some(operand) = (0..node.named_child_count())
                .find_map(|i| node.named_child(i))
            {
                match operand.kind() {
                    "composite_literal" => {
                        if let Some(type_node) = operand.child_by_field_name("type") {
                            CallArg::Ident(node_text(&type_node, src))
                        } else {
                            CallArg::Other
                        }
                    }
                    "identifier" => CallArg::Ident(node_text(&operand, src)),
                    _ => CallArg::Other,
                }
            } else {
                CallArg::Other
            }
        }
        // `&[]User{}` — slice address-of. Operand is composite_literal
        // whose type is a slice_type.
        "composite_literal" => {
            if let Some(type_node) = node.child_by_field_name("type") {
                let raw = node_text(&type_node, src);
                CallArg::Ident(raw)
            } else {
                CallArg::Other
            }
        }
        "int_literal" | "float_literal" | "imaginary_literal" => {
            CallArg::Literal(node_text(node, src))
        }
        "true" | "false" | "nil" => CallArg::Literal(node.kind().to_string()),
        // `args...` — variadic spread. The single named child is the spread
        // operand; recurse on it.
        "variadic_argument" => {
            let inner = node
                .named_child(0)
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Spread { expr: Box::new(inner) }
        }
        // `m[k]` — subscript / index access. Recurse on operand and index.
        "index_expression" => {
            let container = node
                .child_by_field_name("operand")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let index = node
                .child_by_field_name("index")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::IndexAccess {
                container: Box::new(container),
                index: Box::new(index),
            }
        }
        // `left op right` — binary expression. Capture operator text and
        // recurse on both operands.
        "binary_expression" => {
            let op = node
                .child_by_field_name("operator")
                .map(|n| node_text(&n, src))
                .unwrap_or_default();
            let left = node
                .child_by_field_name("left")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            let right = node
                .child_by_field_name("right")
                .map(|n| extract_arg(&n, src, depth + 1))
                .unwrap_or(CallArg::Other);
            CallArg::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            }
        }
        _ => CallArg::Other,
    }
}

fn strip_go_string(raw: &str) -> String {
    // Interpreted strings use `"..."` (with escapes); raw strings use `` `...` ``.
    raw.trim_matches('"').trim_matches('`').to_string()
}

// ---------------------------------------------------------------------------
// Body traversal — refs + local variable symbols
// ---------------------------------------------------------------------------

/// Walk a function/method body, extracting both:
///   1. All call/composite-literal/type-assertion refs (via `extract_refs_from_body`)
///   2. Local variable symbols from `:=` declarations and `for range` clauses
///
/// `enclosing_idx` is the index of the enclosing function/method symbol.
/// `qualified_prefix` is the qualified name of the enclosing function (used as
/// the scope_path for the emitted Variable symbols).
pub(super) fn extract_body_with_symbols(
    body: &Node,
    source: &str,
    enclosing_idx: usize,
    qualified_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    extract_body_with_symbols_inner(body, source, enclosing_idx, qualified_prefix, symbols, refs);
}

fn extract_body_with_symbols_inner(
    node: &Node,
    source: &str,
    enclosing_idx: usize,
    qualified_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `:=` short variable declaration
            "short_var_declaration" => {
                super::statements::extract_short_var_decl(
                    &child,
                    source,
                    symbols,
                    refs,
                    Some(enclosing_idx),
                    qualified_prefix,
                    enclosing_idx,
                );
                // Extract fields from any anonymous struct types on the RHS.
                extract_inline_struct_fields_from_rhs(
                    &child,
                    source,
                    symbols,
                    refs,
                    Some(enclosing_idx),
                    qualified_prefix,
                );
            }

            // `var x Type = val` — explicit var declaration inside a function body.
            "var_declaration" => {
                super::statements::extract_const_var_decl(
                    &child,
                    source,
                    symbols,
                    refs,
                    Some(enclosing_idx),
                    qualified_prefix,
                    "var",
                    "var_spec",
                );
                // Extract fields from any anonymous struct types on the RHS.
                extract_inline_struct_fields_from_rhs(
                    &child,
                    source,
                    symbols,
                    refs,
                    Some(enclosing_idx),
                    qualified_prefix,
                );
                extract_refs_from_body(&child, source, enclosing_idx, refs);
            }

            // `const x = val` — explicit const declaration inside a function body.
            "const_declaration" => {
                super::statements::extract_const_var_decl(
                    &child,
                    source,
                    symbols,
                    refs,
                    Some(enclosing_idx),
                    qualified_prefix,
                    "const",
                    "const_spec",
                );
                extract_refs_from_body(&child, source, enclosing_idx, refs);
            }

            // `type inner struct { X int }` — type declaration inside a function body.
            // Extracts Struct/Interface/TypeAlias symbols and their fields.
            "type_declaration" => {
                super::types::extract_type_declaration(
                    &child,
                    source,
                    symbols,
                    refs,
                    Some(enclosing_idx),
                    qualified_prefix,
                );
            }

            // `for i, v := range slice { ... }`
            "for_statement" => {
                extract_for_range_vars(&child, source, enclosing_idx, qualified_prefix, symbols, refs);
                // Recurse into the body block.
                let mut fc = child.walk();
                for fc_child in child.children(&mut fc) {
                    if fc_child.kind() == "block" {
                        extract_body_with_symbols_inner(
                            &fc_child, source, enclosing_idx, qualified_prefix, symbols, refs,
                        );
                    }
                }
                // Also extract plain refs from the whole for_statement.
                extract_refs_from_body(&child, source, enclosing_idx, refs);
            }

            // `select { case msg := <-ch: ... }` — variables in communication_case
            "select_statement" => {
                let mut sc = child.walk();
                for case_child in child.children(&mut sc) {
                    if case_child.kind() == "communication_case" {
                        // Look for a short_var_declaration inside the case header.
                        let mut cc = case_child.walk();
                        for cc_child in case_child.children(&mut cc) {
                            if cc_child.kind() == "short_var_declaration" {
                                super::statements::extract_short_var_decl(
                                    &cc_child,
                                    source,
                                    symbols,
                                    refs,
                                    Some(enclosing_idx),
                                    qualified_prefix,
                                    enclosing_idx,
                                );
                            }
                        }
                        // Recurse into case body.
                        extract_body_with_symbols_inner(
                            &case_child, source, enclosing_idx, qualified_prefix, symbols, refs,
                        );
                    } else if case_child.kind() == "default_case" {
                        extract_body_with_symbols_inner(
                            &case_child, source, enclosing_idx, qualified_prefix, symbols, refs,
                        );
                    }
                }
                // Also extract plain refs.
                extract_refs_from_body(&child, source, enclosing_idx, refs);
            }

            // All other nodes: extract refs and recurse for nested symbols.
            _ => {
                extract_refs_from_body(&child, source, enclosing_idx, refs);
                extract_body_with_symbols_inner(
                    &child, source, enclosing_idx, qualified_prefix, symbols, refs,
                );
            }
        }
    }
}

/// Extract loop variables from `for i, v := range slice { ... }`.
///
/// Tree-sitter-go shape:
/// ```text
/// for_statement
///   for_clause / for_range_clause
///     left:  expression_list   → identifiers
///     right: expression        → the slice/map/channel
///   block
/// ```
///
/// `for_range_clause` has `left` and `right` field names in the grammar.
fn extract_for_range_vars(
    for_node: &Node,
    source: &str,
    enclosing_idx: usize,
    qualified_prefix: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    use super::helpers::{go_visibility, qualify, scope_from_prefix};

    let mut cursor = for_node.walk();
    for child in for_node.children(&mut cursor) {
        if child.kind() != "range_clause" {
            continue;
        }

        let left = match child.child_by_field_name("left") {
            Some(n) => n,
            None => continue,
        };

        // Collect identifiers from the left side.
        let mut lc = left.walk();
        for ident in left.children(&mut lc) {
            if ident.kind() != "identifier" {
                continue;
            }
            let name = node_text(&ident, source);
            if name == "_" {
                continue;
            }
            let qualified_name = qualify(&name, qualified_prefix);
            let visibility = go_visibility(&name);

            symbols.push(ExtractedSymbol {
                name,
                qualified_name,
                kind: crate::types::SymbolKind::Variable,
                visibility,
                start_line: ident.start_position().row as u32,
                end_line: ident.end_position().row as u32,
                start_col: ident.start_position().column as u32,
                end_col: ident.end_position().column as u32,
                signature: None,
                doc_comment: None,
                scope_path: scope_from_prefix(qualified_prefix),
                parent_index: Some(enclosing_idx),
                byte_offset: 0,
                            declared_type: None,
                return_type: None,
                param_types: Vec::new(),
                generic_params: Vec::new(),
});
        }

        // Extract refs from the right-hand side (the range expression).
        if let Some(right) = child.child_by_field_name("right") {
            extract_refs_from_body(&right, source, enclosing_idx, refs);
        }
    }
}

// ---------------------------------------------------------------------------
// Inline struct field extraction from declaration RHS
// ---------------------------------------------------------------------------

/// Walk the RHS children of a `short_var_declaration` or `var_declaration` and
/// extract field symbols from any anonymous `struct_type` nodes found there.
/// Skips plain `identifier` children (the LHS names).
fn extract_inline_struct_fields_from_rhs(
    node: &Node,
    source: &str,
    symbols: &mut Vec<crate::types::ExtractedSymbol>,
    refs: &mut Vec<crate::types::ExtractedRef>,
    parent_index: Option<usize>,
    qualified_prefix: &str,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if !child.is_named() {
            continue;
        }
        if child.kind() == "identifier" {
            continue;
        }
        super::types::extract_inline_struct_fields(
            &child,
            source,
            symbols,
            refs,
            parent_index,
            qualified_prefix,
        );
    }
}
