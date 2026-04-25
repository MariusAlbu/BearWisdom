// =============================================================================
// csharp/helpers.rs  —  Shared utilities for the C# extractor
// =============================================================================

use crate::types::{DbMappingSource, ExtractedDbSet, ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::Node;

pub(super) fn node_text(node: Node, src: &[u8]) -> String {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()])
        .unwrap_or("")
        .to_string()
}

pub(super) fn detect_visibility(node: &Node, _src: &[u8]) -> Option<Visibility> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // tree-sitter-c-sharp wraps each modifier keyword in a `modifier` node.
        if child.kind() == "modifier" {
            let mut mc = child.walk();
            for kw in child.children(&mut mc) {
                match kw.kind() {
                    "public" => return Some(Visibility::Public),
                    "private" => return Some(Visibility::Private),
                    "protected" => return Some(Visibility::Protected),
                    "internal" => return Some(Visibility::Internal),
                    _ => {}
                }
            }
        }
    }
    None
}

pub(super) fn has_modifier(node: &Node, keyword: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifier" {
            let mut mc = child.walk();
            if child.children(&mut mc).any(|kw| kw.kind() == keyword) {
                return true;
            }
        }
    }
    false
}

pub(super) const TEST_ATTRIBUTES: &[&str] = &["Test", "Fact", "Theory", "TestMethod", "TestCase"];

pub(super) fn has_test_attribute(node: &Node, src: &[u8]) -> bool {
    let mut outer = node.walk();
    for child in node.children(&mut outer) {
        if child.kind() == "attribute_list" {
            let mut al = child.walk();
            for attr in child.children(&mut al) {
                if attr.kind() == "attribute" {
                    if let Some(name_node) = attr.child_by_field_name("name") {
                        let name = node_text(name_node, src);
                        if TEST_ATTRIBUTES.contains(&name.as_str()) {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// Collect consecutive `///` doc-comment siblings immediately before `node`.
pub(super) fn extract_doc_comment(node: &Node, src: &[u8]) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut sib = node.prev_sibling();
    while let Some(s) = sib {
        if s.kind() == "comment" {
            let text = node_text(s, src);
            if text.trim_start().starts_with("///") {
                lines.push(text);
                sib = s.prev_sibling();
                continue;
            }
        }
        break;
    }
    if lines.is_empty() {
        return None;
    }
    lines.reverse();
    Some(lines.join("\n"))
}

pub(super) fn build_method_signature(node: &Node, src: &[u8]) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    // Build directly into one buffer. The old path did
    // `format!(...).trim().to_string()` — two allocations per method.
    // On Smartstore's ~50k methods that's 100k redundant allocations.
    let ret = node.child_by_field_name("returns").map(|t| node_text(t, src));
    let type_params = node.child_by_field_name("type_parameters").map(|tp| node_text(tp, src));
    let params = node.child_by_field_name("parameters").map(|p| node_text(p, src));

    let mut sig = String::with_capacity(128);
    if let Some(r) = ret.as_deref() {
        let t = r.trim();
        if !t.is_empty() {
            sig.push_str(t);
            sig.push(' ');
        }
    }
    sig.push_str(&node_text(name_node, src));
    if let Some(tp) = type_params.as_deref() {
        sig.push_str(tp);
    }
    if let Some(p) = params.as_deref() {
        sig.push_str(p);
    }
    Some(sig)
}

pub(super) fn find_child_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    // Short-circuit on first match; the old path collected every child
    // into a Vec<Node> before searching, wasting an allocation per call.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

/// Returns true for C# primitive / standard-library type names that are not
/// useful to track as cross-file references.
pub(super) fn is_builtin_type(name: &str) -> bool {
    matches!(
        name,
        "string"
            | "int"
            | "long"
            | "double"
            | "float"
            | "decimal"
            | "bool"
            | "byte"
            | "char"
            | "short"
            | "uint"
            | "ulong"
            | "ushort"
            | "sbyte"
            | "object"
            | "void"
            | "dynamic"
            | "var"
            | "Task"
            | "IActionResult"
            | "ActionResult"
            | "IResult"
            | "Results"
            | "IEnumerable"
            | "IList"
            | "List"
            | "Dictionary"
            | "HashSet"
            | "ICollection"
            | "IReadOnlyList"
            | "IReadOnlyCollection"
            | "Nullable"
    )
}

/// Returns true if this class declaration inherits from DbContext (directly or
/// via a named subclass ending in "Context").
pub(super) fn is_dbcontext_subclass(node: &Node, src: &[u8]) -> bool {
    use super::types::simple_type_name;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "base_list" {
            let mut bc = child.walk();
            for base in child.children(&mut bc) {
                let name = match base.kind() {
                    "identifier" => node_text(base, src),
                    "generic_name" | "qualified_name" => simple_type_name(base, src),
                    _ => continue,
                };
                if name.contains("DbContext") {
                    return true;
                }
            }
        }
    }
    false
}

/// Walk the body of a DbContext class and collect all `DbSet<T>` properties.
pub(super) fn extract_db_sets_from_body(
    body: &Node,
    src: &[u8],
    _scope_tree: &crate::parser::scope_tree::ScopeTree,
    symbols: &[ExtractedSymbol],
    db_sets: &mut Vec<ExtractedDbSet>,
) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "property_declaration" {
            // Check if the type is DbSet<T>.
            if let Some(type_node) = child.child_by_field_name("type") {
                let type_str = node_text(type_node, src);
                if type_str.starts_with("DbSet<") {
                    // Extract T from DbSet<T>.
                    let entity_type = type_str
                        .trim_start_matches("DbSet<")
                        .trim_end_matches('>')
                        .trim()
                        .to_string();

                    // Find the symbol for this property in the symbols vec.
                    let name_node = match child.child_by_field_name("name") {
                        Some(n) => n,
                        None => continue,
                    };
                    let prop_name = node_text(name_node, src);

                    // Find the symbol index (linear scan — fine for small DbContext classes).
                    let prop_sym_idx = symbols
                        .iter()
                        .rposition(|s| s.name == prop_name && s.kind == SymbolKind::Property)
                        .unwrap_or(0);

                    // Determine table name: check for [Table("...")] attribute first.
                    // We'll look on the entity class — that's a cross-file concern, but
                    // we record what we can here.  The connector will enrich this later.
                    let table_name = entity_type.clone(); // convention: plural is applied by connector
                    let source = check_table_attribute_on_property(&child, src)
                        .map(|_| DbMappingSource::Attribute)
                        .unwrap_or(DbMappingSource::Convention);

                    db_sets.push(ExtractedDbSet {
                        property_symbol_index: prop_sym_idx,
                        entity_type,
                        table_name,
                        source,
                    });
                }
            }
        }
    }
}

/// Returns the table name from a [Table("...")] attribute if present.
pub(super) fn check_table_attribute_on_property(node: &Node, src: &[u8]) -> Option<String> {
    use super::calls::attr_route_template;
    let mut outer = node.walk();
    for child in node.children(&mut outer) {
        if child.kind() == "attribute_list" {
            let mut al = child.walk();
            for attr in child.children(&mut al) {
                if attr.kind() == "attribute" {
                    if let Some(name_node) = attr.child_by_field_name("name") {
                        if node_text(name_node, src) == "Table" {
                            return attr_route_template(&attr, src);
                        }
                    }
                }
            }
        }
    }
    None
}
