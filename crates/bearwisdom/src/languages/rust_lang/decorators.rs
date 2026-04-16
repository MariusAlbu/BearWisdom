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
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
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
                target_name: name.clone(),
                kind: EdgeKind::TypeRef,
                line: attr_item.start_position().row as u32,
                module: first_arg,
                chain: None,
            });

            // For #[derive(...)], also extract each derived trait as a TypeRef
            if name == "derive" {
                extract_derive_trait_refs(&attr_item, source, source_symbol_index, refs);
            }
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
                    extract_trait_names_from_token_tree(&ac_child, source, source_symbol_index, refs);
                    break;
                }
            }
            break;
        }
    }
}

/// Recursively extract trait names from a derive token_tree.
///
/// For `(Debug, Clone, Serialize)`, emits TypeRef for each identifier.
fn extract_trait_names_from_token_tree(
    tt: &Node,
    source: &str,
    source_symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    let mut cursor = tt.walk();
    for child in tt.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let trait_name = node_text(&child, source);
                if !trait_name.is_empty() && trait_name != "," {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: trait_name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
            "scoped_identifier" => {
                // e.g., `serde::Serialize` — extract the full path
                let full_name = node_text(&child, source);
                if !full_name.is_empty() {
                    refs.push(ExtractedRef {
                        source_symbol_index,
                        target_name: full_name,
                        kind: EdgeKind::TypeRef,
                        line: child.start_position().row as u32,
                        module: None,
                        chain: None,
                    });
                }
            }
            "token_tree" => {
                // Nested parens; recurse
                extract_trait_names_from_token_tree(&child, source, source_symbol_index, refs);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Derive synthesis
// ---------------------------------------------------------------------------

/// Synthesize method/function symbols for each `#[derive(...)]` trait on a
/// struct or enum.  The synthesized symbols are parented to the type symbol
/// (`parent_sym_idx`) and placed under `qualified_prefix` (i.e. the type's
/// own qualified name as prefix).
///
/// Only the derives with deterministic, widely-used methods are synthesized:
///
/// | Derive         | Synthesized symbols                          |
/// |----------------|----------------------------------------------|
/// | Clone          | clone → Method                               |
/// | Copy           | (marker trait — no methods)                  |
/// | Debug          | fmt → Method                                 |
/// | Default        | default → Function (associated)              |
/// | PartialEq / Eq | eq, ne → Method                              |
/// | PartialOrd/Ord | partial_cmp, cmp → Method                    |
/// | Hash           | hash → Method                                |
/// | Serialize      | serialize → Method                           |
/// | Deserialize    | deserialize → Function                       |
/// | From / Into    | from, into → Function / Method               |
/// | AsRef / AsMut  | as_ref, as_mut → Method                      |
pub(super) fn synthesize_derive_methods(
    item_node: &Node,
    source: &str,
    parent_sym_idx: usize,
    qualified_prefix: &str, // the type's qualified name, e.g. "crate.models.User"
    line: u32,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let derives = collect_derive_names(item_node, source);
    if derives.is_empty() {
        return;
    }

    for derive_name in &derives {
        // Normalize: strip path prefix (e.g. "serde::Serialize" → "Serialize")
        let bare = derive_name.rsplit("::").next().unwrap_or(derive_name.as_str());

        let methods: &[(&str, SymbolKind)] = match bare {
            "Clone" => &[("clone", SymbolKind::Method)],
            "Copy" => &[],
            "Debug" | "Display" => &[("fmt", SymbolKind::Method)],
            "Default" => &[("default", SymbolKind::Function)],
            "PartialEq" => &[("eq", SymbolKind::Method), ("ne", SymbolKind::Method)],
            "Eq" => &[],
            "PartialOrd" => &[("partial_cmp", SymbolKind::Method)],
            "Ord" => &[("cmp", SymbolKind::Method)],
            "Hash" => &[("hash", SymbolKind::Method)],
            "Serialize" => &[("serialize", SymbolKind::Method)],
            "Deserialize" => &[("deserialize", SymbolKind::Function)],
            "DeserializeOwned" => &[],
            "From" => &[("from", SymbolKind::Function)],
            "Into" => &[("into", SymbolKind::Method)],
            "AsRef" => &[("as_ref", SymbolKind::Method)],
            "AsMut" => &[("as_mut", SymbolKind::Method)],
            "Error" => &[("source", SymbolKind::Method), ("description", SymbolKind::Method)],
            _ => &[],
        };

        for &(method_name, kind) in methods {
            let qualified_name = if qualified_prefix.is_empty() {
                method_name.to_string()
            } else {
                format!("{qualified_prefix}.{method_name}")
            };

            // Only add if not already present (avoid duplicates when impl block exists).
            if symbols.iter().any(|s| s.qualified_name == qualified_name) {
                continue;
            }

            symbols.push(ExtractedSymbol {
                name: method_name.to_string(),
                qualified_name,
                kind,
                visibility: Some(Visibility::Public),
                start_line: line,
                end_line: line,
                start_col: 0,
                end_col: 0,
                signature: Some(format!("/* synthesized from #[derive({bare})] */")),
                doc_comment: None,
                scope_path: Some(qualified_prefix.to_string()),
                parent_index: Some(parent_sym_idx),
            });
        }
    }
}

/// Collect the bare derive trait names from preceding `attribute_item` siblings.
/// Returns e.g. `["Clone", "Debug", "serde::Serialize"]`.
pub(super) fn collect_derive_names(item_node: &Node, source: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut sib = item_node.prev_sibling();
    while let Some(s) = sib {
        if s.kind() != "attribute_item" {
            break;
        }
        // Parse the attribute_item for `derive(...)` forms.
        let mut ac = s.walk();
        for attr_child in s.children(&mut ac) {
            if attr_child.kind() != "attribute" {
                continue;
            }
            let mut inner = attr_child.walk();
            let mut children_iter = attr_child.children(&mut inner);
            let name_node = match children_iter.next() {
                Some(n) => n,
                None => continue,
            };
            let attr_name = node_text(&name_node, source);
            if attr_name != "derive" {
                continue;
            }
            // Found #[derive(...)]; collect identifiers from the token_tree.
            for tt_child in children_iter {
                if tt_child.kind() == "token_tree" {
                    collect_idents_from_token_tree(&tt_child, source, &mut result);
                }
            }
        }
        sib = s.prev_sibling();
    }
    result
}

fn collect_idents_from_token_tree(tt: &Node, source: &str, out: &mut Vec<String>) {
    let mut cursor = tt.walk();
    for child in tt.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = node_text(&child, source);
                if !name.is_empty() {
                    out.push(name);
                }
            }
            "scoped_identifier" => {
                let name = node_text(&child, source);
                if !name.is_empty() {
                    out.push(name);
                }
            }
            "token_tree" => {
                collect_idents_from_token_tree(&child, source, out);
            }
            _ => {}
        }
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
