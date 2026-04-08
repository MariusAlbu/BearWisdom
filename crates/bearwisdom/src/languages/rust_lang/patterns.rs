// =============================================================================
// rust/patterns.rs  —  Pattern extraction for match/if-let/while-let
//
// Extracts:
//   - TypeRef edges for enum variant patterns  (User::Admin, Some, None)
//   - Variable symbols for pattern bindings    (the `user` in `Some(user)`)
//   - TypeRef edges for where-clause trait bounds
// =============================================================================

use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Match expression
// ---------------------------------------------------------------------------

/// Extract TypeRef and Variable symbols from a `match_expression` node.
///
/// Tree-sitter Rust structure:
///   match_expression → match_block → match_arm → match_pattern → <actual pattern>
///
/// The `match_arm` may have a `pattern` field OR a `match_pattern` child.
pub(super) fn extract_match_patterns(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    walk_for_match_arms(node, source, source_symbol_index, symbols, refs);
}

fn walk_for_match_arms(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "match_arm" => {
                extract_match_arm_patterns(&child, source, source_symbol_index, symbols, refs);
            }
            // Recurse into match_block and other containers
            _ => {
                walk_for_match_arms(&child, source, source_symbol_index, symbols, refs);
            }
        }
    }
}

fn extract_match_arm_patterns(
    arm: &Node,
    source: &str,
    source_symbol_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = arm.walk();
    for child in arm.children(&mut cursor) {
        match child.kind() {
            // `match_pattern` is the node kind for the pattern in an arm
            "match_pattern" => {
                // The pattern content is inside match_pattern
                let mut pc = child.walk();
                for pat_child in child.children(&mut pc) {
                    extract_pattern(&pat_child, source, source_symbol_index, symbols, refs);
                }
            }
            // Direct `pattern` field access as fallback
            _ if arm.child_by_field_name("pattern").map(|p| p.id() == child.id()).unwrap_or(false) => {
                extract_pattern(&child, source, source_symbol_index, symbols, refs);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// if_let / while_let
// ---------------------------------------------------------------------------

/// Extract Variable symbols from a `let_condition` node (`if let Pat = val`).
///
/// Tree-sitter-Rust structure for `if let Some(user) = find_user() { ... }`:
///   if_expression
///     let_condition
///       let "let"
///       <pattern node>    ← what we extract
///       = "="
///       <value expression>
///
/// The pattern is the first non-keyword named child after `let`.
pub(super) fn extract_let_condition_pattern(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    // Try field name "pattern" first
    if let Some(pattern) = node.child_by_field_name("pattern") {
        extract_pattern(&pattern, source, source_symbol_index, symbols, refs);
        return;
    }
    // Fallback: walk children, skip `let` and `=` tokens, take the first pattern node
    let mut cursor = node.walk();
    let mut past_let = false;
    for child in node.children(&mut cursor) {
        match child.kind() {
            "let" => {
                past_let = true;
            }
            "=" => break, // past the pattern, into the value
            _ if past_let && child.is_named() => {
                extract_pattern(&child, source, source_symbol_index, symbols, refs);
                break;
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Type parameter bounds  (<T: Clone + Send>)
// ---------------------------------------------------------------------------

/// Extract TypeRef edges from `type_parameters` — the `<T: Clone + Send>` section.
///
/// Tree-sitter Rust uses `constrained_type_parameter` for bounded params.
/// However, for `fn f<T>() where T: Clone` the bounds only appear in `where_clause`.
/// This function handles inline bounds like `fn f<T: Clone>()`.
pub(super) fn extract_type_param_bounds(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `constrained_type_parameter`: `T: Clone + Send`
            "constrained_type_parameter" | "type_parameter" => {
                if let Some(bounds) = child.child_by_field_name("bounds") {
                    extract_trait_bounds(&bounds, source, source_symbol_index, refs);
                } else {
                    // Bounds may be direct children (trait_bounds node)
                    let mut cc = child.walk();
                    for gc in child.children(&mut cc) {
                        if gc.kind() == "trait_bounds" {
                            extract_trait_bounds(&gc, source, source_symbol_index, refs);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// where clause trait bounds
// ---------------------------------------------------------------------------

/// Extract TypeRef edges for every trait bound in a `where_clause` node.
///
/// `where T: Clone + Send` → TypeRefs for Clone, Send.
pub(super) fn extract_where_clause(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "where_predicate" {
            extract_where_predicate(&child, source, source_symbol_index, refs);
        }
    }
}

fn extract_where_predicate(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Try field name "bounds" first
    if let Some(bounds) = node.child_by_field_name("bounds") {
        extract_trait_bounds(&bounds, source, source_symbol_index, refs);
        return;
    }
    // Fallback: walk children for trait_bounds node
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "trait_bounds" {
            extract_trait_bounds(&child, source, source_symbol_index, refs);
        }
    }
}

/// Also used for inline `<T: Clone + Send>` type parameter bounds.
pub(super) fn extract_trait_bounds(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // A bare trait name: `Clone`
            "type_identifier" | "identifier" => {
                let name = node_text(&child, source);
                if !name.is_empty() {
                    refs.push(make_typeref(source_symbol_index, name, child.start_position().row as u32));
                }
            }
            // A path like `serde::Serialize` or `std::marker::Send`
            "scoped_type_identifier" | "scoped_identifier" => {
                // Only take the leaf trait name
                let name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, source))
                    .unwrap_or_else(|| {
                        let text = node_text(&child, source);
                        text.rsplit("::").next().unwrap_or(&text).to_string()
                    });
                if !name.is_empty() {
                    refs.push(make_typeref(source_symbol_index, name, child.start_position().row as u32));
                }
            }
            // Generic like `Iterator<Item = T>` — extract the base name
            "generic_type" => {
                if let Some(base) = child.child_by_field_name("type") {
                    let name = node_text(&base, source);
                    if !name.is_empty() {
                        refs.push(make_typeref(source_symbol_index, name, child.start_position().row as u32));
                    }
                }
            }
            // Higher-ranked `for<'a> Trait<'a>` — recurse
            "higher_ranked_trait_bound" => {
                extract_trait_bounds(&child, source, source_symbol_index, refs);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Pattern walker
// ---------------------------------------------------------------------------

/// Recursively walk a pattern node and emit:
/// - TypeRef for variant/type names (identifier in scoped context, or constants)
/// - Variable for binding identifiers
fn extract_pattern(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        // `Some`, `None`, `Ok`, `Err`, bare enum variant — emit TypeRef
        "identifier" => {
            let name = node_text(node, source);
            // Skip `_` wildcard and lowercase bindings (those are variables, not types).
            // Enum variants in Rust are always PascalCase or SCREAMING_SNAKE_CASE.
            if is_type_name(&name) {
                refs.push(make_typeref(source_symbol_index, name, node.start_position().row as u32));
            } else if name != "_" && !name.is_empty() {
                symbols.push(make_variable(name, node, source_symbol_index));
            }
        }

        // `Type::Variant` or `crate::Error`
        "scoped_identifier" => {
            let name = node_text(node, source);
            // Take the full scoped name as the TypeRef target
            refs.push(make_typeref(source_symbol_index, name, node.start_position().row as u32));
        }

        // `Struct { field, .. }` — emit TypeRef for struct name, Variable for fields
        "struct_pattern" => {
            if let Some(type_node) = node.child_by_field_name("type") {
                let type_name = node_text(&type_node, source);
                refs.push(make_typeref(source_symbol_index, type_name, type_node.start_position().row as u32));
            }
            // Walk the field patterns for binding variables
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "field_pattern" {
                    if let Some(pat) = child.child_by_field_name("pattern") {
                        extract_pattern(&pat, source, source_symbol_index, symbols, refs);
                    } else {
                        // `{ name, .. }` — the identifier itself is the binding
                        let mut fc = child.walk();
                        for fc_child in child.children(&mut fc) {
                            if fc_child.kind() == "identifier" {
                                let name = node_text(&fc_child, source);
                                if !name.is_empty() && name != ".." {
                                    symbols.push(make_variable(name, &fc_child, source_symbol_index));
                                }
                            }
                        }
                    }
                }
            }
        }

        // `Variant(binding)` or `Some(x)` — emit TypeRef for variant, Variable for inner
        "tuple_struct_pattern" => {
            if let Some(type_node) = node.child_by_field_name("type") {
                let type_name = node_text(&type_node, source);
                refs.push(make_typeref(source_symbol_index, type_name, type_node.start_position().row as u32));
            }
            // Recurse into pattern arguments (the bindings inside the parens)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "(" | ")" | "," => {}
                    _ if child.kind() != "type" => {
                        extract_pattern(&child, source, source_symbol_index, symbols, refs);
                    }
                    _ => {}
                }
            }
        }

        // `(a, b, c)` tuple patterns
        "tuple_pattern" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_pattern(&child, source, source_symbol_index, symbols, refs);
            }
        }

        // `ref x`, `mut x`, `ref mut x`
        "ref_pattern" | "mut_pattern" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = node_text(&child, source);
                    if !name.is_empty() && name != "_" && name != "ref" && name != "mut" {
                        symbols.push(make_variable(name, &child, source_symbol_index));
                    }
                } else {
                    extract_pattern(&child, source, source_symbol_index, symbols, refs);
                }
            }
        }

        // `x @ pattern` — the binding `x` is a Variable
        "captured_pattern" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(&name_node, source);
                if !name.is_empty() && name != "_" {
                    symbols.push(make_variable(name, &name_node, source_symbol_index));
                }
            }
            if let Some(pattern) = node.child_by_field_name("pattern") {
                extract_pattern(&pattern, source, source_symbol_index, symbols, refs);
            }
        }

        // `[a, b, ..]` slice patterns
        "slice_pattern" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_pattern(&child, source, source_symbol_index, symbols, refs);
            }
        }

        // `&pattern`
        "reference_pattern" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() != "&" {
                    extract_pattern(&child, source, source_symbol_index, symbols, refs);
                }
            }
        }

        // or_pattern: `Pat1 | Pat2`
        "or_pattern" | "alternative_pattern" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_pattern(&child, source, source_symbol_index, symbols, refs);
            }
        }

        // Literals and wildcards — nothing to emit
        "_" | "integer_literal" | "float_literal" | "string_literal"
        | "boolean_literal" | "char_literal" | "wildcard_pattern" => {}

        // Recurse into everything else (e.g. `remaining_field_pattern`, `..`)
        other => {
            if other != ".." && other != "," && other != "|" && other != "(" && other != ")" {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    extract_pattern(&child, source, source_symbol_index, symbols, refs);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Heuristic: a name starting with uppercase is a type/variant name, not a binding.
fn is_type_name(name: &str) -> bool {
    name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
}

fn make_typeref(source_symbol_index: usize, name: String, line: u32) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index,
        target_name: name,
        kind: EdgeKind::TypeRef,
        line,
        module: None,
        chain: None,
    }
}

fn make_variable(name: String, node: &Node, parent_index: usize) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.clone(),
        qualified_name: name,
        kind: SymbolKind::Variable,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: Some(parent_index),
    }
}


// ---------------------------------------------------------------------------
// Trait supertrait bounds  →  Inherits edges
// ---------------------------------------------------------------------------

/// Extract `Inherits` edges for supertrait bounds on a `trait_item` node.
///
/// `trait Foo: Bar + Baz` — the `bounds` field on `trait_item` contains the
/// supertrait constraints. These are semantic parent traits, so we emit
/// `EdgeKind::Inherits` rather than `TypeRef`.
///
/// The rules spec says:
///   trait_item → bounds field → trait_bounds → type_identifier → Inherits
pub(super) fn extract_supertrait_bounds(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // The `bounds` field contains a `trait_bounds` node.
    if let Some(bounds) = node.child_by_field_name("bounds") {
        emit_inherits_from_trait_bounds(&bounds, source, source_symbol_index, refs);
    }
}

/// Walk a `trait_bounds` node and emit `Inherits` for each bound trait.
fn emit_inherits_from_trait_bounds(
    node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_identifier" | "identifier" => {
                let name = node_text(&child, source);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::Inherits,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
            "scoped_type_identifier" | "scoped_identifier" => {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| node_text(&n, source))
                    .unwrap_or_else(|| {
                        let text = node_text(&child, source);
                        text.rsplit("::").next().unwrap_or(&text).to_string()
                    });
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::Inherits,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
            "generic_type" => {
                if let Some(base) = child.child_by_field_name("type") {
                    let name = node_text(&base, source);
                    if !name.is_empty() {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::Inherits,
                            line: child.start_position().row as u32,
                            module: None,
                            chain: None,
                        });
                    }
                }
            }
            "higher_ranked_trait_bound" => {
                emit_inherits_from_trait_bounds(&child, source, source_symbol_index, refs);
            }
            // Skip `Sized`, `?Sized`, lifetime bounds (`'a`), and `+` separators
            _ => {}
        }
    }
}
