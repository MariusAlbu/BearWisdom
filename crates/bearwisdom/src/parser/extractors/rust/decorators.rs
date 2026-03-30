// =============================================================================
// rust/decorators.rs  —  Attribute extraction for Rust
//
// Rust attribute forms:
//   #[derive(Debug, Clone, Serialize)]  → attribute with token_tree args
//   #[test]                             → bare attribute
//   #[cfg(test)]                        → attribute with token_tree
//   #[route("/api/users")]              → attribute with string arg
//   #[serde::rename_all = "camelCase"]  → path attribute
//
// Tree-sitter shape: `attribute_item` nodes appear as siblings *before* the
// item they annotate (struct_item, enum_item, fn_item, impl_item, etc.).
//
//   attribute_item
//     "#["
//     attribute
//       identifier "derive"         ← or path like "serde::rename_all"
//       token_tree "(Debug, Clone)" ← optional arguments
//     "]"
//
// Strategy: given the annotated item node, walk *previous siblings* collecting
// consecutive `attribute_item` nodes (stop at the first non-attribute sibling).
// =============================================================================

use super::helpers::node_text;
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

/// Emit one `ExtractedRef` per attribute attached to `item_node`.
///
/// `item_node` is the struct/enum/fn/trait/mod item.  Attributes are its
/// preceding siblings in the CST.
pub(super) fn extract_decorators(
    item_node: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut collected: Vec<Node> = Vec::new();
    let mut sib = item_node.prev_sibling();
    while let Some(s) = sib {
        if s.kind() == "attribute_item" {
            collected.push(s);
        } else {
            break;
        }
        sib = s.prev_sibling();
    }
    // collected is in reverse order (closest sibling first); reverse to top-to-bottom.
    collected.reverse();

    for attr_item in collected {
        if let Some((name, first_arg)) = parse_attribute_item(&attr_item, source) {
            refs.push(ExtractedRef {
                source_symbol_index,
                target_name: name,
                kind: EdgeKind::TypeRef,
                line: attr_item.start_position().row as u32,
                module: first_arg,
                chain: None,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Parse a single `attribute_item`
// ---------------------------------------------------------------------------

fn parse_attribute_item(attr_item: &Node, source: &str) -> Option<(String, Option<String>)> {
    // The `attribute_item` wraps an `attribute` node.
    let mut cursor = attr_item.walk();
    for child in attr_item.children(&mut cursor) {
        if child.kind() == "attribute" {
            return parse_attribute(&child, source);
        }
    }
    None
}

fn parse_attribute(attr: &Node, source: &str) -> Option<(String, Option<String>)> {
    // First child of `attribute` is the path/identifier (the attribute name).
    // Optional second child is a `token_tree` with the arguments.
    let mut cursor = attr.walk();
    let mut children = attr.children(&mut cursor);

    let name_node = children.next()?;
    let name = match name_node.kind() {
        "identifier" => node_text(&name_node, source),
        // path like `serde::rename_all`; use the last segment for a concise name.
        "scoped_identifier" => {
            let full = node_text(&name_node, source);
            full.rsplit("::").next().unwrap_or(&full).to_string()
        }
        _ => return None,
    };

    if name.is_empty() {
        return None;
    }

    // Look for the first string literal in the token_tree argument.
    let first_arg = children
        .find(|c| c.kind() == "token_tree")
        .and_then(|tt| extract_first_string_from_token_tree(&tt, source));

    Some((name, first_arg))
}

/// Recursively scan a `token_tree` for the first string literal.
fn extract_first_string_from_token_tree(tt: &Node, source: &str) -> Option<String> {
    let mut cursor = tt.walk();
    for child in tt.children(&mut cursor) {
        match child.kind() {
            "string_literal" | "raw_string_literal" => {
                let raw = node_text(&child, source);
                let stripped = raw
                    .trim_start_matches("r#\"")
                    .trim_start_matches("r\"")
                    .trim_end_matches("\"#")
                    .trim_matches('"')
                    .to_string();
                if !stripped.is_empty() {
                    return Some(stripped);
                }
            }
            "token_tree" => {
                // Nested parens: recurse.
                if let Some(s) = extract_first_string_from_token_tree(&child, source) {
                    return Some(s);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::extract;
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
    fn derive_attribute() {
        let src = "#[derive(Debug, Clone)]\nstruct Point { x: i32, y: i32 }";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "derive"), "refs: {dr:?}");
    }

    #[test]
    fn test_attribute_on_fn() {
        let src = "#[test]\nfn it_works() {}";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "test"), "refs: {dr:?}");
    }

    #[test]
    fn attribute_with_string_arg() {
        let src = r#"#[route("/api/users")]
fn users() {}"#;
        let dr = decorator_refs(src);
        let found = dr.iter().find(|(n, _)| n == "route");
        assert!(found.is_some(), "refs: {dr:?}");
        assert_eq!(found.unwrap().1, Some("/api/users".to_string()));
    }

    #[test]
    fn multiple_attributes() {
        let src = "#[derive(Debug)]\n#[serde(rename_all = \"camelCase\")]\nstruct Cfg {}";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "derive"), "refs: {dr:?}");
        assert!(dr.iter().any(|(n, _)| n == "serde"), "refs: {dr:?}");
    }

    #[test]
    fn attribute_on_enum() {
        let src = "#[derive(Debug, PartialEq)]\nenum Status { Active, Inactive }";
        let dr = decorator_refs(src);
        assert!(dr.iter().any(|(n, _)| n == "derive"), "refs: {dr:?}");
    }
}
