// =============================================================================
// dart/decorators.rs  —  Annotation and cascade extraction for Dart
//
// Dart annotation forms:
//   @override                        → annotation (name: identifier)
//   @JsonSerializable()              → annotation (name + annotation_arguments)
//   @Route('/api')                   → annotation with string arg
//
// Tree-sitter (dart 0.1):
//   annotation
//     name: identifier | qualified
//     annotation_arguments? (optional)
//
// Annotations appear as children of:
//   class_declaration, class_member, enum_declaration, extension_declaration,
//   formal_parameter, library_import, library_export.
//
// Cascade expressions:
//   object..method1()..field = value
//   Tree-sitter: expression_statement / return_statement / …
//     → cascade_section (multiple)
//       → cascade_selector: identifier | call_expression | …
//
// Null-aware operators:
//   user?.name  — tree-sitter: postfix_expression → selector (with `?.`)
//   value ?? default — if_null_expression
//   These are handled by extracting type references from the sub-expressions.
//
// NOTE: Dart uses &str (UTF-8 source string) not &[u8] — this file follows
//       the existing Dart extractor convention.
// =============================================================================

use super::helpers::{node_text, qualify, scope_from_prefix};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Decorators (annotations)
// ---------------------------------------------------------------------------

/// Emit one `ExtractedRef` per `@annotation` found as a direct child of `node`.
///
/// In the Dart grammar annotations appear as named children of the enclosing
/// declaration node (not wrapped in a `modifiers` group).
pub(super) fn extract_decorators(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "annotation" {
            emit_annotation(&child, src, source_symbol_index, refs);
        }
    }
}

/// Also extract annotations from `class_member` wrappers inside a class body.
///
/// The Dart grammar wraps each class member (method/field declaration) in a
/// `class_member` node that may have `annotation` children before the actual
/// `method_signature` or `declaration` children.
#[allow(dead_code)]
pub(super) fn extract_class_member_decorators(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() != "class_member" {
        return;
    }
    extract_decorators(node, src, source_symbol_index, refs);
}

fn emit_annotation(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if let Some(name) = annotation_name(node, src) {
        let first_arg = extract_first_string_arg(node, src);
        refs.push(ExtractedRef {
            source_symbol_index,
            target_name: name,
            kind: EdgeKind::TypeRef,
            line: node.start_position().row as u32,
            module: first_arg,
            chain: None,
            byte_offset: 0,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
});
    }
}

/// Extract the annotation name.
///
/// Dart grammar: annotation has a `name` field of type `identifier` or `qualified`.
fn annotation_name(node: &Node, src: &str) -> Option<String> {
    // Prefer the grammar field.
    if let Some(name_node) = node.child_by_field_name("name") {
        return Some(name_from_identifier_or_qualified(&name_node, src));
    }
    // Fallback: first identifier child.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" | "qualified" => {
                return Some(name_from_identifier_or_qualified(&child, src));
            }
            _ => {}
        }
    }
    None
}

fn name_from_identifier_or_qualified(node: &Node, src: &str) -> String {
    let text = node_text(*node, src);
    // For `qualified` (e.g. `prefix.Name`) take the last segment.
    text.rsplit('.').next().unwrap_or(&text).to_string()
}

fn extract_first_string_arg(annotation_node: &Node, src: &str) -> Option<String> {
    let mut cursor = annotation_node.walk();
    for child in annotation_node.children(&mut cursor) {
        if child.kind() == "annotation_arguments" {
            let mut ac = child.walk();
            for arg in child.children(&mut ac) {
                // String literals have kind `string_literal` or `string`.
                match arg.kind() {
                    "string_literal" | "string" => {
                        return strip_string(node_text(arg, src));
                    }
                    // Named argument: `@Route(path: '/api')` — look inside.
                    "named_argument" | "argument" => {
                        let mut nc = arg.walk();
                        for inner in arg.children(&mut nc) {
                            if inner.kind() == "string_literal" || inner.kind() == "string" {
                                return strip_string(node_text(inner, src));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

fn strip_string(raw: String) -> Option<String> {
    let s = raw.trim_matches('"').trim_matches('\'').to_string();
    if s.is_empty() { None } else { Some(s) }
}

// ---------------------------------------------------------------------------
// Cascade expression call extraction
// ---------------------------------------------------------------------------

/// Extract each cascaded call target from a cascade chain.
///
/// ```dart
/// object..method1()..method2()..field = value
/// ```
///
/// Tree-sitter: expression_statement → cascade_section* children.
/// Each `cascade_section` contains a `cascade_selector` which holds the
/// identifier or call target.
///
/// We emit one `Calls` ref per method call segment in the chain.
pub(super) fn extract_cascade_calls(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "cascade_section" {
            extract_cascade_section(&child, src, source_symbol_index, refs);
        } else {
            extract_cascade_calls(&child, src, source_symbol_index, refs);
        }
    }
}

fn extract_cascade_section(
    node: &Node,
    src: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "cascade_selector" => {
                // cascade_selector may contain an identifier directly.
                let mut sc = child.walk();
                for inner in child.children(&mut sc) {
                    if inner.kind() == "identifier" {
                        let name = node_text(inner, src);
                        if !name.is_empty() {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::Calls,
                                line: inner.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                                                            namespace_segments: Vec::new(),
                                                            call_args: Vec::new(),
});
                        }
                        break;
                    }
                }
            }
            // Direct identifier at the cascade level.
            "identifier" => {
                let name = node_text(child, src);
                if !name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::Calls,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                                            namespace_segments: Vec::new(),
                                            call_args: Vec::new(),
});
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Null-aware type reference extraction
// ---------------------------------------------------------------------------

/// Extract `TypeRef` edges from null-aware expressions.
///
/// ```dart
/// user?.name        → TypeRef to receiver type (limited — we emit the identifier)
/// value ?? default  → no TypeRef needed (it's value-level, not type-level)
/// ```
///
/// In practice the most useful signal is the `?` propagation in type
/// annotations. We don't try to infer receiver types (that requires
/// type inference); instead we leave it to the call extractor. This
/// function is a no-op placeholder for future implementation.
pub(super) fn extract_null_aware_refs(
    _node: &Node,
    _src: &str,
    _source_symbol_index: usize,
    _refs: &mut Vec<ExtractedRef>,
) {
    // No-op: null-aware type refs require type inference which is out of scope
    // for a structural extractor. The call extractor already handles `?.method`.
}

// ---------------------------------------------------------------------------
// Annotation symbols (standalone @annotations as Variable symbols)
// ---------------------------------------------------------------------------

/// Emit `Variable` symbols for metadata annotations attached to a declaration.
/// Used when we want the annotation itself to appear as a searchable symbol.
///
/// In practice annotations are already emitted as TypeRef edges by
/// `extract_decorators`; this is a no-op placeholder for future extension.
#[allow(dead_code)]
pub(super) fn extract_annotation_symbols(
    _node: &Node,
    _src: &str,
    _parent_index: usize,
    _parent_qname: &str,
    _symbols: &mut Vec<ExtractedSymbol>,
) {
    // Annotations are represented as TypeRef edges, not symbols.
}

// ---------------------------------------------------------------------------
// Helper: push a simple Variable symbol (used by callers if needed)
// ---------------------------------------------------------------------------

/// Push a `Variable` symbol with the given name at the given location.
#[allow(dead_code)]
pub(super) fn push_variable(
    name: String,
    qualified_prefix: &str,
    node: &Node,
    src: &str,
    parent_index: Option<usize>,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let qualified_name = qualify(&name, qualified_prefix);
    symbols.push(ExtractedSymbol {
        name,
        qualified_name,
        kind: SymbolKind::Variable,
        visibility: Some(Visibility::Public),
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path: scope_from_prefix(qualified_prefix),
        parent_index,
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::extract::extract;
    use crate::types::EdgeKind;

    fn type_refs(source: &str) -> Vec<(String, Option<String>)> {
        extract(source)
            .refs
            .into_iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| (r.target_name, r.module))
            .collect()
    }

    #[test]
    fn json_serializable_annotation_on_class() {
        // Class-level annotations are direct children of class_declaration.
        let src = "@JsonSerializable()\nclass User {}";
        let refs = type_refs(src);
        assert!(
            refs.iter().any(|(n, _)| n == "JsonSerializable"),
            "JsonSerializable not found; refs: {refs:?}"
        );
    }

    #[test]
    fn route_annotation_on_class() {
        let src = "@Route('/api')\nclass ApiService {}";
        let refs = type_refs(src);
        assert!(
            refs.iter().any(|(n, _)| n == "Route"),
            "Route not found; refs: {refs:?}"
        );
    }

    #[test]
    fn multiple_annotations_on_class() {
        let src = "@sealed\n@deprecated\nclass Old {}";
        let refs = type_refs(src);
        assert!(refs.iter().any(|(n, _)| n == "sealed"), "refs: {refs:?}");
        assert!(refs.iter().any(|(n, _)| n == "deprecated"), "refs: {refs:?}");
    }
}

