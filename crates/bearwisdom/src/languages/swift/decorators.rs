// =============================================================================
// swift/decorators.rs  —  Attribute and pattern extraction for Swift
//
// Swift attribute forms:
//   @objc                     → attribute (direct child of declaration)
//   @IBOutlet                 → attribute
//   @available(iOS 13, *)    → attribute with arguments
//
// Tree-sitter (swift 0.7):
//   function_declaration / class_declaration etc. have `attribute` as direct
//   named children (not wrapped in a `modifiers` node). The attribute's first
//   named child is the name as a `simple_identifier` or `user_type`.
//
// Guard / switch patterns:
//   guard_statement → condition: value_binding_pattern (let/var binding)
//   switch_statement → switch_entry → switch_pattern → value_binding_pattern |
//                                                      case_class_pattern | …
//
// Protocol/extension:
//   extension_declaration already extracted by symbols.rs; here we emit
//   TypeRef from extension to the extended type (conformance constraints).
// =============================================================================

use super::helpers::{find_child_by_kind, node_text};
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Decorators (attributes)
// ---------------------------------------------------------------------------

/// Emit one `ExtractedRef` per `@attribute` found as a direct child of `node`
/// or nested inside a `modifiers` child.
///
/// The Swift grammar places attributes in two ways depending on context:
///   - As direct named children of class/struct/function declarations
///   - Inside a `modifiers` node (observed empirically with tree-sitter-swift 0.7)
pub(super) fn extract_decorators(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "attribute" => {
                emit_attribute(&child, src, source_symbol_index, refs);
            }
            "modifiers" => {
                let mut mc = child.walk();
                for attr in child.children(&mut mc) {
                    if attr.kind() == "attribute" {
                        emit_attribute(&attr, src, source_symbol_index, refs);
                    }
                }
            }
            _ => {}
        }
    }
}

fn emit_attribute(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if let Some(name) = attribute_name(node, src) {
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: None,
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
});
    }
}

/// Emit a TypeRef for a single `attribute` node directly.
///
/// Unlike `extract_decorators` (which walks a declaration node's children for
/// attributes), this operates directly on the attribute node itself.
pub(super) fn emit_single_attribute(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    emit_attribute(node, src, source_symbol_index, refs);
}

/// Extract the attribute name from an `attribute` node.
///
/// The Swift grammar puts the name as a `simple_identifier` or `user_type`
/// child immediately after the `@` token.
fn attribute_name(node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "simple_identifier" | "identifier" | "type_identifier" => {
                let t = node_text(child, src);
                if !t.is_empty() && t != "@" {
                    return Some(t);
                }
            }
            "user_type" => {
                // Swift: user_type → type_identifier (e.g. "objc")
                if let Some(name) = name_from_user_type(&child, src) {
                    return Some(name);
                }
            }
            _ => {}
        }
    }
    None
}

fn name_from_user_type(node: &Node, src: &[u8]) -> Option<String> {
    let mut last: Option<String> = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Swift uses type_identifier as the leaf (not simple_identifier)
            "type_identifier" | "simple_identifier" | "identifier" => {
                let t = node_text(child, src);
                if !t.is_empty() {
                    last = Some(t);
                }
            }
            "type_name" => {
                let mut tc = child.walk();
                for inner in child.children(&mut tc) {
                    match inner.kind() {
                        "type_identifier" | "simple_identifier" | "identifier" => {
                            last = Some(node_text(inner, src));
                            break;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    last
}

// ---------------------------------------------------------------------------
// Guard-let bindings
// ---------------------------------------------------------------------------

/// Extract bound identifier names from `guard_statement` conditions.
///
/// ```swift
/// guard let user = optionalUser else { return }
/// ```
///
/// Tree-sitter: guard_statement has `bound_identifier` fields (simple_identifier).
/// We emit a TypeRef for each bound identifier name so the graph knows what
/// types are being unwrapped.
pub(super) fn extract_guard_bindings(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() != "guard_statement" {
        return;
    }
    // bound_identifier fields give us the names of the let-bound variables.
    // Scan conditions for value_binding_pattern → type annotation (user_type).
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_binding_pattern_type(&child, src, source_symbol_index, refs);
    }
}

fn extract_binding_pattern_type(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "value_binding_pattern" => {
            // Emit TypeRef for the pattern — find a type annotation sibling.
            // The grammar puts the type in a `type_annotation` sibling of the
            // value_binding_pattern in the containing expression.
            // We just record the presence of a binding; types come from explicit
            // `as` casts or `is` checks in the condition list.
        }
        "check_expression" => {
            // `x is SomeType` — emit TypeRef for SomeType.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "user_type" || child.kind() == "simple_identifier" {
                    if let Some(name) = if child.kind() == "user_type" {
                        name_from_user_type(&child, src)
                    } else {
                        Some(node_text(child, src))
                    } {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::TypeRef,
                            line: child.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
});
                    }
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_binding_pattern_type(&child, src, source_symbol_index, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Switch / case patterns
// ---------------------------------------------------------------------------

/// Extract type references from `switch_statement` case patterns.
///
/// ```swift
/// switch value {
///     case .admin(let level): ...
///     case let x as Admin: ...
/// }
/// ```
///
/// Tree-sitter: switch_statement → switch_entry → switch_pattern → pattern
pub(super) fn extract_switch_patterns(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() != "switch_statement" {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "switch_entry" {
            let mut ec = child.walk();
            for item in child.children(&mut ec) {
                if item.kind() == "switch_pattern" || item.kind() == "pattern" {
                    extract_pattern_type_refs(&item, src, source_symbol_index, refs);
                }
            }
        }
    }
}

fn extract_pattern_type_refs(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "switch_pattern" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_pattern_type_refs(&child, src, source_symbol_index, refs);
            }
        }
        "pattern" => {
            // pattern may contain: is_expression, as_pattern, tuple_pattern, etc.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_pattern_type_refs(&child, src, source_symbol_index, refs);
            }
        }
        // `is SomeType` inside a pattern.
        "check_expression" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "user_type" {
                    if let Some(name) = name_from_user_type(&child, src) {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::TypeRef,
                            line: child.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
});
                    }
                } else if child.kind() == "simple_identifier" {
                    let name = node_text(child, src);
                    if !name.is_empty() && name != "is" {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: name,
                            kind: EdgeKind::TypeRef,
                            line: child.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
});
                    }
                }
            }
        }
        // Enum member match `.admin` — navigation expression.
        "navigation_expression" => {
            if let Some(rhs) = find_child_by_kind(node, "navigation_suffix") {
                let mut nc = rhs.walk();
                for inner in rhs.children(&mut nc) {
                    if inner.kind() == "simple_identifier" {
                        refs.push(ExtractedRef {
                            source_symbol_index,
                            target_name: node_text(inner, src),
                            kind: EdgeKind::TypeRef,
                            line: inner.start_position().row as u32,
                            module: None,
                            chain: None,
                            byte_offset: 0,
                                                    namespace_segments: Vec::new(),
});
                    }
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_pattern_type_refs(&child, src, source_symbol_index, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Extension conformance
// ---------------------------------------------------------------------------

/// Emit TypeRef edges for types named in an `extension_declaration`'s
/// `type_inheritance_clause` (protocol conformances added by the extension).
///
/// ```swift
/// extension Array where Element: Comparable { }
/// ```
///
/// The primary extended type is already handled by `push_extension` in
/// symbols.rs as the symbol name; here we emit refs for any conformance
/// types in the `where` clause.
pub(super) fn extract_extension_conformances(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() != "extension_declaration" {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "type_constraints" | "where_clause" => {
                let mut wc = child.walk();
                for constraint in child.children(&mut wc) {
                    // inheritance_constraint / equality_constraint
                    let mut cc = constraint.walk();
                    for inner in constraint.children(&mut cc) {
                        if inner.kind() == "user_type" {
                            if let Some(name) = name_from_user_type(&inner, src) {
                                refs.push(ExtractedRef {
                                    source_symbol_index,
                                    target_name: name,
                                    kind: EdgeKind::TypeRef,
                                    line: inner.start_position().row as u32,
                                    module: None,
                                    chain: None,
                                    byte_offset: 0,
                                                                    namespace_segments: Vec::new(),
});
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::extract::extract;
    use crate::types::EdgeKind;

    fn type_refs(source: &str) -> Vec<String> {
        extract(source)
            .refs
            .into_iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| r.target_name)
            .collect()
    }

    #[test]
    fn objc_attribute() {
        let src = "class C {\n    @objc func doThing() {}\n}";
        let refs = type_refs(src);
        assert!(refs.contains(&"objc".to_string()), "refs: {refs:?}");
    }

    #[test]
    fn multiple_attributes_on_property() {
        let src = "class C {\n    @IBOutlet @objc var button: UIButton?\n}";
        let refs = type_refs(src);
        assert!(refs.contains(&"IBOutlet".to_string()), "refs: {refs:?}");
        assert!(refs.contains(&"objc".to_string()), "refs: {refs:?}");
    }
}

