// =============================================================================
// scala/decorators.rs  —  Annotation and pattern extraction for Scala
//
// Scala annotation forms:
//   @Service                         → annotation (name: type_identifier)
//   @RequestMapping("/api")          → annotation (name + arguments)
//
// Tree-sitter (scala 0.25):
//   annotation
//     name: type_identifier | stable_type_identifier | generic_type | …
//     arguments: arguments? (optional)
//
//   Annotations appear as direct children of class/trait/object/def nodes.
//   They are NOT wrapped in a `modifiers` node in tree-sitter-scala.
//
// Match expression:
//   match_expression
//     value: expression
//     body: case_block | indented_cases
//       case_clause
//         pattern: _pattern (case_class_pattern → type_identifier | type)
//
// Case class constructor params:
//   class_definition → class_parameters → class_parameter (name, type)
// =============================================================================

use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

// ---------------------------------------------------------------------------
// Decorators
// ---------------------------------------------------------------------------

/// Emit one `ExtractedRef` per annotation on `node`.
///
/// `node` should be a type/function definition node. In tree-sitter-scala
/// annotations appear as direct named children of the declaration.
pub(super) fn extract_decorators(
    node: &Node,
    src: &[u8],
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

fn emit_annotation(
    node: &Node,
    src: &[u8],
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
        });
    }
}

/// Extract the annotation name from an `annotation` node.
///
/// Scala grammar: annotation has a `name` field which is one of:
///   type_identifier, stable_type_identifier, generic_type, …
fn annotation_name(node: &Node, src: &[u8]) -> Option<String> {
    // Try the `name` field first.
    if let Some(name_node) = node.child_by_field_name("name") {
        return extract_type_name(&name_node, src);
    }
    // Fallback: first named child.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.is_named() {
            if let Some(name) = extract_type_name(&child, src) {
                return Some(name);
            }
        }
    }
    None
}

fn extract_type_name(node: &Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "type_identifier" => Some(node_text(*node, src)),
        "stable_type_identifier" => {
            // last identifier in the chain
            let text = node_text(*node, src);
            text.rsplit('.').next().map(str::to_string)
        }
        "generic_type" => {
            // generic_type → stable_id or type_identifier + type_arguments
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "type_identifier" => return Some(node_text(child, src)),
                    "stable_type_identifier" => {
                        let text = node_text(child, src);
                        return text.rsplit('.').next().map(str::to_string);
                    }
                    _ => {}
                }
            }
            None
        }
        "singleton_type" => {
            // singleton_type → stable_id (often `object.method`)
            let text = node_text(*node, src);
            text.rsplit('.').next().map(str::to_string)
        }
        _ => None,
    }
}

fn extract_first_string_arg(annotation_node: &Node, src: &[u8]) -> Option<String> {
    let args = annotation_node.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    for child in args.children(&mut cursor) {
        if child.kind() == "string" || child.kind() == "string_literal"
            || child.kind() == "interpolated_string"
        {
            return strip_string(node_text(child, src));
        }
    }
    None
}

fn strip_string(raw: String) -> Option<String> {
    let s = raw.trim_matches('"').trim_matches('\'').to_string();
    if s.is_empty() { None } else { Some(s) }
}

// ---------------------------------------------------------------------------
// Match expression — pattern matching
// ---------------------------------------------------------------------------

/// Extract TypeRef edges for types named in `case_class_pattern` inside a
/// `match_expression`.
///
/// ```scala
/// x match {
///     case Admin(level) => …
///     case s: Student   => …
/// }
/// ```
///
/// Tree-sitter: match_expression → body (case_block | indented_cases) →
///              case_clause (pattern: _pattern → case_class_pattern | typed_pattern)
pub(super) fn extract_match_patterns(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if node.kind() != "match_expression" {
        return;
    }
    if let Some(body) = node.child_by_field_name("body") {
        walk_case_clauses(&body, src, source_symbol_index, refs);
    } else {
        // indented_cases — direct children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "case_block" || child.kind() == "indented_cases" {
                walk_case_clauses(&child, src, source_symbol_index, refs);
            }
        }
    }
}

fn walk_case_clauses(
    body: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "case_clause" {
            if let Some(pattern) = child.child_by_field_name("pattern") {
                extract_pattern_refs(&pattern, src, source_symbol_index, refs);
            }
        }
    }
}

fn extract_pattern_refs(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "case_class_pattern" => {
            // type field → type_identifier | stable_type_identifier
            if let Some(type_node) = node.child_by_field_name("type") {
                if let Some(name) = extract_type_name(&type_node, src) {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: name,
                        kind: EdgeKind::TypeRef,
                        line: type_node.start_position().row as u32,
                        module: None,
                        chain: None,
                        byte_offset: 0,
                    });
                }
            }
        }
        "typed_pattern" => {
            // e.g. `s: Student` — the type is in a `type` child.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "type_identifier" | "stable_type_identifier" | "generic_type" => {
                        if let Some(name) = extract_type_name(&child, src) {
                            refs.push(ExtractedRef {
                                source_symbol_index,
                                target_name: name,
                                kind: EdgeKind::TypeRef,
                                line: child.start_position().row as u32,
                                module: None,
                                chain: None,
                                byte_offset: 0,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        "alternative_pattern" | "tuple_pattern" => {
            // Recurse into sub-patterns.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_pattern_refs(&child, src, source_symbol_index, refs);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                extract_pattern_refs(&child, src, source_symbol_index, refs);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Case class constructor parameters
// ---------------------------------------------------------------------------

/// Extract `Property` symbols for the parameters of a `case class`.
///
/// ```scala
/// case class User(name: String, email: String)
/// ```
///
/// Tree-sitter: class_definition → class_parameters → class_parameter
///   class_parameter has `name` (identifier) and `type` fields.
///
/// Only emits when the class has `case` in its modifiers.
pub(super) fn extract_case_class_params(
    node: &Node,
    src: &[u8],
    parent_index: usize,
    parent_qname: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    if node.kind() != "class_definition" {
        return;
    }
    // Verify `case` modifier.
    if !is_case_class(node, src) {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "class_parameters" {
            let mut pc = child.walk();
            for param in child.children(&mut pc) {
                if param.kind() == "class_parameter" {
                    push_class_param(&param, src, parent_index, parent_qname, symbols);
                }
            }
        }
    }
}

fn is_case_class(node: &Node, src: &[u8]) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            let text = node_text(child, src);
            if text.contains("case") {
                return true;
            }
        }
        // `case` may appear as a standalone keyword child.
        if child.kind() == "case" {
            return true;
        }
    }
    // Fallback: check the raw text prefix.
    let text = node_text(*node, src);
    text.trim_start().starts_with("case ")
}

fn push_class_param(
    node: &Node,
    src: &[u8],
    parent_index: usize,
    parent_qname: &str,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let name = match node.child_by_field_name("name") {
        Some(n) => node_text(n, src),
        None => return,
    };
    let ty = node
        .child_by_field_name("type")
        .map(|t| format!(": {}", node_text(t, src)))
        .unwrap_or_default();

    let qualified_name = if parent_qname.is_empty() {
        name.clone()
    } else {
        format!("{parent_qname}.{name}")
    };

    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name,
        kind: SymbolKind::Property,
        visibility: None,
        start_line: node.start_position().row as u32,
        end_line: node.end_position().row as u32,
        start_col: node.start_position().column as u32,
        end_col: node.end_position().column as u32,
        signature: Some(format!("val {name}{ty}")),
        doc_comment: None,
        scope_path: if parent_qname.is_empty() { None } else { Some(parent_qname.to_string()) },
        parent_index: Some(parent_index),
    });
}

// ---------------------------------------------------------------------------
// For comprehension call extraction (delegated to call walker)
// ---------------------------------------------------------------------------

/// Recurse into `for_expression` / `for_comprehension` bodies and extract
/// any `call_expression` nodes found there. We delegate to the existing
/// `extract_calls_from_body` so as not to duplicate call-walking logic.
///
/// The caller (mod.rs) passes the for-expression node; this function simply
/// hands it to the call extractor.
#[allow(dead_code)]
pub(super) fn extract_for_comprehension_calls(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Reuse Scala's generic call extractor on the entire for-expression.
    super::calls::extract_calls_from_body(node, src, source_symbol_index, refs);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::extract::extract;
    use crate::types::{EdgeKind, SymbolKind};

    fn type_refs(source: &str) -> Vec<(String, Option<String>)> {
        extract(source)
            .refs
            .into_iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| (r.target_name, r.module))
            .collect()
    }

    #[test]
    fn annotation_on_class() {
        let src = "@Service\nclass UserService {}";
        let dr = type_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "Service"), "refs: {dr:?}");
    }

    #[test]
    fn annotation_with_route() {
        let src = r#"@RequestMapping("/api")
class Api {}"#;
        let dr = type_refs(src);
        let found = dr.iter().find(|(n, _)| n == "RequestMapping");
        assert!(found.is_some(), "RequestMapping not found; refs: {dr:?}");
    }

    #[test]
    fn multiple_annotations() {
        let src = "@Service\n@Transactional\nclass Svc {}";
        let dr = type_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "Service"), "refs: {dr:?}");
        assert!(dr.iter().any(|(n, _)| n == "Transactional"), "refs: {dr:?}");
    }

    #[test]
    fn match_case_class_pattern() {
        let src = r#"
object M {
  def check(x: Any) = x match {
    case Admin(level) => level
    case _            => 0
  }
}
"#;
        let dr = type_refs(src);
        assert!(
            dr.iter().any(|(n, _)| n == "Admin"),
            "Admin TypeRef not found; refs: {dr:?}"
        );
    }

    #[test]
    fn case_class_params_extracted() {
        let src = "case class User(name: String, email: String)";
        let r = extract(src);
        assert!(
            r.symbols.iter().any(|s| s.name == "name" && s.kind == SymbolKind::Property),
            "param 'name' not extracted; symbols: {:?}",
            r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
        );
        assert!(
            r.symbols.iter().any(|s| s.name == "email" && s.kind == SymbolKind::Property),
            "param 'email' not extracted"
        );
    }
}

