// =============================================================================
// csharp/calls_narrowing.rs  —  TypeRef emission from is/switch/cast expressions
// =============================================================================

use super::calls::is_csharp_keyword;
use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

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
pub(super) fn extract_is_expression_refs(
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
                        refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                            source_symbol_index,
                            target_name: type_name,
                            kind: EdgeKind::TypeRef,
                            line: inner.start_position().row as u32,
                            col: 0,
                            module: None,
                            chain: None,
                            byte_offset: inner.start_byte() as u32,
                                                    namespace_segments: Vec::new(),
                                                    call_args: Vec::new(),
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
            refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                source_symbol_index,
                target_name: type_name,
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
                    refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                        source_symbol_index,
                        target_name: type_name,
                        kind: EdgeKind::TypeRef,
                        line: inner.start_position().row as u32,
                        col: 0,
                        module: None,
                        chain: None,
                        byte_offset: inner.start_byte() as u32,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
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
            refs.push(ExtractedRef { is_import_binding: false, is_reexport: false,
                source_symbol_index,
                target_name: type_name,
                kind: EdgeKind::TypeRef,
                line: type_node.start_position().row as u32,
                col: 0,
                module: None,
                chain: None,
                byte_offset: type_node.start_byte() as u32,
                            namespace_segments: Vec::new(),
                            call_args: Vec::new(),
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
pub(super) fn extract_switch_expression_type_refs(
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
pub(super) fn extract_type_ref_from_cast_type(
    type_node: Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    use super::types::extract_type_refs_from_type_node;
    extract_type_refs_from_type_node(type_node, src, source_symbol_index, refs);
}

