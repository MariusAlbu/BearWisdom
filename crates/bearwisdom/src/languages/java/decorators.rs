// =============================================================================
// java/decorators.rs  —  Annotation extraction for Java
//
// Java annotation forms:
//   @Service                         → marker_annotation (no args)
//   @GetMapping("/users/{id}")       → annotation with string literal arg
//   @Autowired                       → marker_annotation
//   @RequestMapping(value="/users")  → annotation with key=value args
//
// Tree-sitter shapes:
//   marker_annotation
//     "@"
//     identifier "Service"
//
//   annotation
//     "@"
//     identifier "GetMapping"
//     annotation_argument_list
//       "("
//       string_literal '"/users/{id}"'   ← or element_value_pair
//       ")"
//
// We walk the children of a class/method declaration node looking for
// `marker_annotation` and `annotation` siblings that precede the declaration.
// =============================================================================

use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

/// Emit one `ExtractedRef` per annotation attached to `node`.
///
/// `node` should be a `class_declaration`, `interface_declaration`,
/// `enum_declaration`, `method_declaration`, or `constructor_declaration`.
///
/// In tree-sitter-java annotations appear either:
///   1. Inside a `modifiers` child of the declaration (most common)
///   2. As direct children of the declaration (older grammar versions)
pub(super) fn extract_decorators(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // Annotations are usually wrapped in a `modifiers` node.
            "modifiers" => {
                let mut mc = child.walk();
                for ann in child.children(&mut mc) {
                    emit_annotation(&ann, src, source_symbol_index, refs);
                }
            }
            // Direct annotation children (fallback for some grammar versions).
            "marker_annotation" | "annotation" => {
                emit_annotation(&child, src, source_symbol_index, refs);
            }
            _ => {}
        }
    }
}

fn emit_annotation(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    match node.kind() {
        "marker_annotation" => {
            if let Some(name) = annotation_name(node, src) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: None,
                    chain: None,
                });
            }
        }
        "annotation" => {
            if let Some(name) = annotation_name(node, src) {
                let first_arg = extract_first_string_arg(node, src);
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: node.start_position().row as u32,
                    module: first_arg,
                    chain: None,
                });
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the annotation name from a `marker_annotation` or `annotation` node.
///
/// Tree-sitter stores the name as an `identifier` or `scoped_identifier` child.
fn annotation_name(node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" | "scoped_identifier" => {
                let name = node_text(child, src);
                if !name.is_empty() {
                    return Some(name);
                }
            }
            _ => {}
        }
    }
    None
}

/// Return the text of the first string literal inside `annotation_argument_list`.
///
/// Handles both positional (`@GetMapping("/users")`) and named
/// (`@RequestMapping(value="/users", method=GET)`) forms; in the latter we
/// take the first string encountered.
fn extract_first_string_arg(annotation_node: &Node, src: &[u8]) -> Option<String> {
    let args = annotation_node
        .children(&mut annotation_node.walk())
        .find(|c| c.kind() == "annotation_argument_list")?;

    let mut cursor = args.walk();
    for child in args.children(&mut cursor) {
        match child.kind() {
            "string_literal" => {
                return strip_java_string(node_text(child, src));
            }
            // element_value_pair: key = value
            "element_value_pair" => {
                let mut evc = child.walk();
                for ev in child.children(&mut evc) {
                    if ev.kind() == "string_literal" {
                        return strip_java_string(node_text(ev, src));
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Strip surrounding double quotes from a Java string literal text.
fn strip_java_string(raw: String) -> Option<String> {
    let stripped = raw.trim_matches('"').to_string();
    if stripped.is_empty() {
        None
    } else {
        Some(stripped)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::extract::extract;
    use crate::types::EdgeKind;

    fn decorator_refs(source: &str) -> Vec<(String, Option<String>)> {
        extract(source)
            .refs
            .into_iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| (r.target_name, r.module))
            .collect()
    }

    #[test]
    fn marker_annotation_on_class() {
        let src = "@Service\npublic class UserService {}";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "Service"), "refs: {dr:?}");
    }

    #[test]
    fn annotation_with_route_arg() {
        let src = r#"public class C {
    @GetMapping("/users/{id}")
    public User get() { return null; }
}"#;
        let dr = decorator_refs(src);
        let found = dr.iter().find(|(n, _)| n == "GetMapping");
        assert!(found.is_some(), "refs: {dr:?}");
        assert_eq!(found.unwrap().1, Some("/users/{id}".to_string()));
    }

    #[test]
    fn multiple_annotations() {
        let src = "@Service\n@Transactional\npublic class Svc {}";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "Service"), "refs: {dr:?}");
        assert!(dr.iter().any(|(n, _)| n == "Transactional"), "refs: {dr:?}");
    }

    #[test]
    fn annotation_no_args() {
        let src = "public class C {\n    @Override\n    public void run() {}\n}";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "Override"), "refs: {dr:?}");
    }
}
