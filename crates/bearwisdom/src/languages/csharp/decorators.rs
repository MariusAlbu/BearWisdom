// =============================================================================
// csharp/decorators.rs  —  Attribute extraction for C#
//
// C# attribute forms:
//   [ApiController]                → attribute_list → attribute (no args)
//   [HttpGet("{id}")]              → attribute_list → attribute + argument_list
//   [Route("api/[controller]")]    → attribute_list → attribute + argument_list
//   [Authorize(Roles = "Admin")]   → attribute_list → attribute + named arg
//
// Tree-sitter shape:
//   attribute_list
//     "[" "["
//     attribute
//       name "ApiController"  ← or identifier
//       attribute_argument_list
//         "(" "("
//         attribute_argument
//           string_literal '"{id}"'
//         ")" ")"
//     "]" "]"
//
// Attributes appear as child nodes of the declaration node before the keywords.
// We walk all children of the declaration node collecting `attribute_list` nodes.
//
// Note: csharp/calls.rs already extracts HTTP route info into `ExtractedRoute`.
// This module emits the complementary `EdgeKind::TypeRef` refs so that all
// attributes appear in the general cross-reference graph (decorator edges).
// =============================================================================

use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

/// Emit one `ExtractedRef` per attribute on a class/method declaration node.
pub(super) fn extract_decorators(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "attribute_list" {
            extract_from_attribute_list(&child, src, source_symbol_index, refs);
        }
    }

    // For methods, also extract attributes on parameters and return values.
    extract_parameter_and_return_attributes(node, src, source_symbol_index, refs);
}

/// Extract attributes from method parameters and return value.
///
/// In C#, attributes can appear on parameters: `void M([Required] int x) { }`
/// and on return values: `[return: NotNull] string Get() { }`
fn extract_parameter_and_return_attributes(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Look for parameter_list (method/constructor parameters).
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "parameter_list" => {
                // Iterate all parameters.
                let mut pl_cursor = child.walk();
                for param in child.children(&mut pl_cursor) {
                    if param.kind() == "parameter" {
                        // Parameters may have attribute_list children.
                        let mut p_cursor = param.walk();
                        for p_child in param.children(&mut p_cursor) {
                            if p_child.kind() == "attribute_list" {
                                extract_from_attribute_list(&p_child, src, source_symbol_index, refs);
                            }
                        }
                    }
                }
            }
            // Return type with attributes: `[return: NotNull] Type MethodName() { }`
            // The return type node may contain attribute_list.
            "return_type" | "type" => {
                let mut rt_cursor = child.walk();
                for rt_child in child.children(&mut rt_cursor) {
                    if rt_child.kind() == "attribute_list" {
                        extract_from_attribute_list(&rt_child, src, source_symbol_index, refs);
                    }
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn extract_from_attribute_list(
    attr_list: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = attr_list.walk();
    // Iterate ALL attributes in the list, not just the first.
    for child in attr_list.children(&mut cursor) {
        if child.kind() == "attribute" {
            if let Some((name, first_arg)) = parse_attribute(&child, src) {
                refs.push(ExtractedRef {
                    source_symbol_index,
                    target_name: name,
                    kind: EdgeKind::TypeRef,
                    line: child.start_position().row as u32,
                    module: first_arg,
                    chain: None,
                    byte_offset: 0,
                });
            }
            // Also extract TypeRefs from attribute arguments (typeof, generic_name, cast, etc.)
            extract_attribute_arg_type_refs(&child, src, source_symbol_index, refs);
        }
    }
}

/// Walk attribute arguments and emit TypeRef edges for type expressions within them.
///
/// Handles:
///   [SomeAttr(typeof(Foo))]          → TypeRef to Foo
///   [SomeAttr(new Foo[])]            → TypeRef to Foo
///   [SomeAttr(SomeEnum.Value)]       → TypeRef to SomeEnum (via generic_name or identifier)
fn extract_attribute_arg_type_refs(
    attr: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = attr.walk();
    for child in attr.children(&mut cursor) {
        if child.kind() == "attribute_argument_list" {
            super::calls::extract_calls_from_body(&child, src, source_symbol_index, refs);
        }
    }
}

fn parse_attribute(attr: &Node, src: &[u8]) -> Option<(String, Option<String>)> {
    // The attribute name is accessed via the `name` field or the first
    // identifier-like child.  tree-sitter-c-sharp uses `name` field.
    let name = if let Some(n) = attr.child_by_field_name("name") {
        let raw = node_text(n, src);
        // Strip generic suffix: `Authorize<T>` → `Authorize`.
        let clean = raw.split('<').next().unwrap_or(&raw).to_string();
        clean
    } else {
        // Fallback: walk children for the first identifier/qualified_name.
        let mut cursor = attr.walk();
        let candidate = attr.children(&mut cursor).find(|c| {
            matches!(c.kind(), "identifier" | "qualified_name" | "generic_name")
        });
        match candidate {
            Some(n) => {
                let raw = node_text(n, src);
                raw.split('<').next().unwrap_or(&raw).to_string()
            }
            None => return None,
        }
    };

    if name.is_empty() {
        return None;
    }

    // Look for the first string literal inside the attribute_argument_list.
    let first_arg = attr
        .children(&mut attr.walk())
        .find(|c| c.kind() == "attribute_argument_list")
        .and_then(|args| extract_first_string(&args, src));

    Some((name, first_arg))
}

/// Return the first string literal text found in an `attribute_argument_list`.
fn extract_first_string(args_node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = args_node.walk();
    for child in args_node.children(&mut cursor) {
        // attribute_argument wraps the actual value expression
        if child.kind() == "attribute_argument" {
            let mut ac = child.walk();
            for val in child.children(&mut ac) {
                if let Some(s) = try_extract_string_from_node(&val, src) {
                    return Some(s);
                }
            }
        }
        // Or a direct string literal at this level
        if let Some(s) = try_extract_string_from_node(&child, src) {
            return Some(s);
        }
    }
    None
}

fn try_extract_string_from_node(node: &Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "string_literal" | "verbatim_string_literal" => {
            let raw = node_text(*node, src);
            // Strip @"..." or "..." → inner content.
            let inner = raw
                .trim_start_matches('@')
                .trim_matches('"')
                .to_string();
            if inner.is_empty() { None } else { Some(inner) }
        }
        _ => None,
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
    fn marker_attribute_on_class() {
        let src = "[ApiController]\npublic class UsersController {}";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "ApiController"), "refs: {dr:?}");
    }

    #[test]
    fn attribute_with_route_arg() {
        let src = "public class C {\n    [HttpGet(\"{id}\")]\n    public User Get(int id) { return null; }\n}";
        let dr = decorator_refs(src);
        let found = dr.iter().find(|(n, _)| n == "HttpGet");
        assert!(found.is_some(), "refs: {dr:?}");
        assert_eq!(found.unwrap().1, Some("{id}".to_string()));
    }

    #[test]
    fn multiple_attributes() {
        let src = "[ApiController]\n[Route(\"api/[controller]\")]\npublic class C {}";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "ApiController"), "refs: {dr:?}");
        assert!(dr.iter().any(|(n, _)| n == "Route"), "refs: {dr:?}");
    }

    #[test]
    fn attribute_no_arg() {
        let src = "public class C {\n    [Authorize]\n    public void Act() {}\n}";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "Authorize"), "refs: {dr:?}");
    }
}
