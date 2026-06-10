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
            // The bare attribute name (`prost`, `serde`, `tokio`, `tracing`, ...)
            // is decorator metadata, not a type or call reference. Emitting it as
            // a TypeRef edge produces unresolved entries with no consumer:
            //   * the resolver only reads EdgeKind::Imports for scope building
            //     (see resolve.rs `build_file_context`).
            //
            // For `#[derive(...)]` we still need the inner trait names — those ARE
            // real type references that participate in inheritance/impl edges.
            if name == "derive" {
                extract_derive_trait_refs(&attr_item, source, source_symbol_index, refs);
                continue;
            }
            // HTTP-method route attributes (actix-web `#[get("/x")]`, Rocket
            // `#[post("/x")]`, etc.) — emit a TypeRef with the URL carried in
            // `module` so the resolver's flow-emission step can lift it to a
            // Consumer HttpCall. Restricted to the canonical verb set so we
            // don't flood unresolved-refs with arbitrary attribute names.
            // Tauri `#[command]` is also handled here (no URL arg required).
            if is_http_verb_attr(&name) {
                let url_or_none = first_arg.as_deref();
                let url_ok = url_or_none.map_or(name == "command", |u| u.starts_with('/'));
                if url_ok {
                    refs.push(ExtractedRef {
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index,
                        target_name: name.clone(),
                        kind: EdgeKind::TypeRef,
                        line: attr_item.start_position().row as u32,
                        col: 0,
                        module: url_or_none.map(String::from),
                        chain: None,
                        byte_offset: attr_item.start_byte() as u32,
                        namespace_segments: Vec::new(),
                        call_args: Vec::new(),
                    });
                }
            }
        }
    }
}

fn is_http_verb_attr(name: &str) -> bool {
    matches!(
        name,
        "get" | "post" | "put" | "patch" | "delete" | "head" | "options" | "route" | "command"
    )
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

/// Extract all trait names from a #[derive(...)] attribute as TypeRef edges.
///
/// For `#[derive(Debug, Clone, Serialize)]`, emits TypeRef for Debug, Clone, Serialize.
fn extract_derive_trait_refs(
    attr_item: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = attr_item.walk();
    for child in attr_item.children(&mut cursor) {
        if child.kind() == "attribute" {
            // Walk the attribute's children for the token_tree argument
            let mut ac = child.walk();
            for ac_child in child.children(&mut ac) {
                if ac_child.kind() == "token_tree" {
                    extract_trait_names_from_token_tree(
                        &ac_child,
                        source,
                        source_symbol_index,
                        refs,
                    );
                    break;
                }
            }
            break;
        }
    }
}

/// Recursively extract trait names from a derive token_tree.
///
/// For `(Debug, Clone, Serialize)`, emits TypeRef for each trait. Handles
/// qualified paths — `prost::Message`, `serde::Deserialize` — that the
/// tree-sitter-rust grammar represents inside derive token_trees as a flat
/// sequence (`identifier "prost"`, `"::"`, `identifier "Message"`) rather
/// than wrapping them in a `scoped_identifier` node. We coalesce those
/// into one ref carrying the full path, so the resolver gets the same
/// shape it would for an explicit `scoped_identifier`.
fn extract_trait_names_from_token_tree(
    tt: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    // Materialize children so we can look ahead for the `::` continuation.
    let mut cursor = tt.walk();
    let children: Vec<Node> = tt.children(&mut cursor).collect();

    let mut i = 0;
    while i < children.len() {
        let child = children[i];
        match child.kind() {
            "identifier" => {
                let mut path = node_text(&child, source);
                if path.is_empty() || path == "," {
                    i += 1;
                    continue;
                }
                let line = child.start_position().row as u32;

                // Coalesce `ident (:: ident)*` produced as flat tokens.
                let mut j = i + 1;
                while j + 1 < children.len()
                    && children[j].kind() == "::"
                    && children[j + 1].kind() == "identifier"
                {
                    path.push_str("::");
                    path.push_str(&node_text(&children[j + 1], source));
                    j += 2;
                }

                refs.push(ExtractedRef {
                    is_import_binding: false,
                    is_reexport: false,
                    source_symbol_index,
                    target_name: path,
                    kind: EdgeKind::TypeRef,
                    line,
                    module: None,
                    chain: None,
                    byte_offset: child.start_byte() as u32,
                    namespace_segments: Vec::new(),
                    call_args: Vec::new(),
                    col: 0,
                });
                i = j;
                continue;
            }
            "scoped_identifier" => {
                // Pre-coalesced by the grammar — emit verbatim.
                let full_name = node_text(&child, source);
                if !full_name.is_empty() {
                    refs.push(ExtractedRef {
                        is_import_binding: false,
                        is_reexport: false,
                        source_symbol_index,
                        target_name: full_name,
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
            }
            "token_tree" => {
                // Nested parens; recurse.
                extract_trait_names_from_token_tree(&child, source, source_symbol_index, refs);
            }
            _ => {}
        }
        i += 1;
    }
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
#[path = "decorators_tests.rs"]
mod tests;
