// =============================================================================
// go/qualified_types.rs — package-qualified type reduction (`pkg.Type`)
//
// Every extraction site that turns a Go type AST node into a ref target needs
// the same two pieces: the bare name Go's own qualified names use (`Type`,
// matching `qualify(name, package)`), and the package qualifier for
// `ExtractedRef::module` attribution — the resolve engine's `ref_module` and
// `package_short_name` rungs bind a bare target back to its package through
// that field. This module is the one place that splits a `qualified_type`
// node and unwraps the container shapes (pointer, slice, map, array,
// channel, generic) that can wrap one, so every call site (params, results,
// fields, embeds, var/const decls, type assertions, type switches, composite
// literals, generic type args) carries the same, complete answer instead of
// each re-deriving its own partial one.
// =============================================================================

use super::helpers::node_text;
use tree_sitter::Node;

/// Split a `qualified_type` node (`pkg.Type`) into its package qualifier and
/// bare type name via the grammar's `package` / `name` fields.
pub(super) fn qualified_type_parts(node: &Node, source: &str) -> Option<(String, String)> {
    let package = node.child_by_field_name("package")?;
    let name = node.child_by_field_name("name")?;
    Some((node_text(&package, source), node_text(&name, source)))
}

/// Reduce a Go type node to `(target_name, module)` for TypeRef / Instantiates /
/// Inherits emission — the bare type name, plus its package qualifier when the
/// node, or the container wrapping it, is a `qualified_type`. Unwraps
/// `pointer_type`, `slice_type`, `map_type`, `array_type`, `channel_type`,
/// `generic_type`, and `parenthesized_type` to the element/value/base type a
/// ref should actually point at.
pub(super) fn go_type_ref_target(node: &Node, source: &str) -> Option<(String, Option<String>)> {
    match node.kind() {
        "type_identifier" => {
            let name = node_text(node, source);
            if name.is_empty() {
                None
            } else {
                Some((name, None))
            }
        }
        "qualified_type" => {
            let (package, name) = qualified_type_parts(node, source)?;
            if name.is_empty() {
                None
            } else {
                Some((name, Some(package)))
            }
        }
        "pointer_type" | "slice_type" => node
            .named_child(0)
            .and_then(|n| go_type_ref_target(&n, source)),
        "map_type" => node
            .child_by_field_name("value")
            .and_then(|n| go_type_ref_target(&n, source)),
        "array_type" => node
            .child_by_field_name("element")
            .and_then(|n| go_type_ref_target(&n, source)),
        "channel_type" => node
            .child_by_field_name("value")
            .and_then(|n| go_type_ref_target(&n, source)),
        // `List[int]` / `pkg.List[int]` (Go 1.18+) — the base is the `type`
        // field, not `name`; it may itself be a `qualified_type`.
        "generic_type" => node
            .child_by_field_name("type")
            .and_then(|n| go_type_ref_target(&n, source)),
        "parenthesized_type" => node
            .named_child(0)
            .and_then(|n| go_type_ref_target(&n, source)),
        _ => None,
    }
}

#[cfg(test)]
#[path = "qualified_types_tests.rs"]
mod tests;
