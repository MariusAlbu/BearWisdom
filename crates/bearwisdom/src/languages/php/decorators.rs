// =============================================================================
// php/decorators.rs  —  PHP 8 attribute extraction
//
// PHP 8 attribute forms:
//   #[Route('/api/users')]         → attribute_group → attribute + arguments
//   #[ORM\Entity]                  → attribute_group → attribute (no args)
//   #[Assert\NotBlank]             → attribute_group → bare attribute
//
// Tree-sitter shape for tree-sitter-php:
//
//   attribute_group
//     "#[" "#["
//     attribute
//       name "Route"              ← or qualified_name like "ORM\Entity"
//       arguments
//         "(" "("
//         argument
//           string "'/api/users'"
//         ")" ")"
//     "]" "]"
//
// In some grammar versions the wrapping node is `attribute_list` containing
// `attribute_group` nodes.  We handle both.
//
// Attributes appear as children of the declaration node before the `class`/
// `function` keyword.
// =============================================================================

use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

/// Emit one `ExtractedRef` per PHP 8 attribute on a class/method node.
pub(super) fn extract_decorators(
    node: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "attribute_group" => {
                extract_from_attribute_group(&child, src, source_symbol_index, refs);
            }
            // Some grammar versions wrap groups in attribute_list
            "attribute_list" => {
                let mut gc = child.walk();
                for grandchild in child.children(&mut gc) {
                    if grandchild.kind() == "attribute_group" {
                        extract_from_attribute_group(&grandchild, src, source_symbol_index, refs);
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

fn extract_from_attribute_group(
    group: &Node,
    src: &[u8],
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = group.walk();
    for child in group.children(&mut cursor) {
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
                                    namespace_segments: Vec::new(),
                                    call_args: Vec::new(),
});
            }
        }
    }
}

fn parse_attribute(attr: &Node, src: &[u8]) -> Option<(String, Option<String>)> {
    // First meaningful child is the attribute name.
    let mut cursor = attr.walk();
    let mut children = attr.children(&mut cursor);

    let name_node = children.next()?;
    let name = match name_node.kind() {
        "name" | "identifier" => node_text(&name_node, src),
        // Qualified like `ORM\Entity` → use last segment as primary name.
        "qualified_name" => {
            let full = node_text(&name_node, src);
            full.rsplit('\\').next().unwrap_or(&full).to_string()
        }
        _ => return None,
    };

    if name.is_empty() {
        return None;
    }

    // Look for `arguments` child for the first string arg.
    let first_arg = children
        .find(|c| c.kind() == "arguments")
        .and_then(|args| extract_first_string_arg(&args, src));

    Some((name, first_arg))
}

/// Return the first string literal from a PHP `arguments` node.
fn extract_first_string_arg(args_node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = args_node.walk();
    for child in args_node.children(&mut cursor) {
        // argument wraps the actual value
        if child.kind() == "argument" {
            let mut ac = child.walk();
            for val in child.children(&mut ac) {
                if let Some(s) = try_extract_php_string(&val, src) {
                    return Some(s);
                }
            }
        }
        // Or a direct string at this level
        if let Some(s) = try_extract_php_string(&child, src) {
            return Some(s);
        }
    }
    None
}

fn try_extract_php_string(node: &Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "string" | "encapsed_string" => {
            let raw = node_text(node, src);
            // Strip surrounding quotes.
            let stripped = raw.trim_matches('"').trim_matches('\'').to_string();
            if stripped.is_empty() { None } else { Some(stripped) }
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
    fn bare_attribute_on_class() {
        let src = "<?php\n#[ORM\\Entity]\nclass User {}";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "Entity"), "refs: {dr:?}");
    }

    #[test]
    fn attribute_with_route_arg() {
        let src = "<?php\n#[Route('/api/users')]\nclass UserController {}";
        let dr = decorator_refs(src);
        let found = dr.iter().find(|(n, _)| n == "Route");
        assert!(found.is_some(), "refs: {dr:?}");
        assert_eq!(found.unwrap().1, Some("/api/users".to_string()));
    }

    #[test]
    fn multiple_attributes() {
        let src = "<?php\n#[ApiController]\n#[Route('/api/users')]\nclass Ctrl {}";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "ApiController"), "refs: {dr:?}");
        assert!(dr.iter().any(|(n, _)| n == "Route"), "refs: {dr:?}");
    }
}
