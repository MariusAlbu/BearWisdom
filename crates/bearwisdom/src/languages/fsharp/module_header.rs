// =============================================================================
// languages/fsharp/module_header.rs — module/namespace header name resolution
// =============================================================================

use tree_sitter::Node;

use super::extract::{is_keyword, node_text};

/// Resolve the dotted path of a `namespace` / `named_module` header node.
///
/// Well-formed headers — `module A.B`, `module internal A.B.C`, headers behind
/// attributes or compiler directives — carry the path in the node's `name`
/// field. A recursive header (`module rec A.B`, `module internal rec A.B`)
/// parses under error recovery: the `name` field mis-binds to the `rec`
/// keyword and the real dotted path lands inside a following
/// `application_expression` sibling. An empty or keyword-valued field is
/// therefore a mis-bind, recovered by scanning the header's other children.
pub(super) fn module_header_name(node: &Node, src: &str) -> String {
    let field = node
        .child_by_field_name("name")
        .map(|n| node_text(&n, src).trim().to_string())
        .unwrap_or_default();
    if !field.is_empty() && !is_keyword(&field) {
        return field;
    }
    recover_module_path(node, src)
}

/// Error-recovery path scan over the header's direct children: skip keyword
/// and modifier tokens (including the mis-bound keyword identifier), then
/// take the first dotted-path-shaped node — either a direct
/// `long_identifier` / `long_identifier_or_op`, or the leading path node of
/// the `application_expression` the parser folded the header into.
fn recover_module_path(node: &Node, src: &str) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "long_identifier" | "long_identifier_or_op" => {
                let t = node_text(&child, src).trim();
                if !is_keyword(t) && is_dotted_module_path(t) {
                    return t.to_string();
                }
            }
            "application_expression" => {
                let Some(first) = child.named_child(0) else {
                    return String::new();
                };
                if matches!(
                    first.kind(),
                    "long_identifier" | "long_identifier_or_op" | "dot_expression"
                ) {
                    let t = node_text(&first, src).trim();
                    if is_dotted_module_path(t) {
                        return t.to_string();
                    }
                }
                return String::new();
            }
            _ => {}
        }
    }
    String::new()
}

/// True when `s` is a dotted identifier chain (`M`, `A.B.C`) — every segment
/// a plain identifier. Guards the recovery scan against grabbing arbitrary
/// expression text.
fn is_dotted_module_path(s: &str) -> bool {
    !s.is_empty()
        && s.split('.').all(|seg| {
            let mut chars = seg.chars();
            matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
                && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '\'')
        })
}
