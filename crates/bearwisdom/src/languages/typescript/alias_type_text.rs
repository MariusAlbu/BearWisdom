// =============================================================================
// languages/typescript/alias_type_text — reading a type expression's name
//
// Classifying an alias RHS asks two different questions: WHICH alias shape is
// this (the classifier's job), and WHAT does this type expression name (this
// module's job). The readers here answer only the second — the head of an
// application, the whole text of a branch that must keep its arguments, a
// tuple element's head, an annotation's head — so the classifier reads as a
// list of shapes rather than a mix of shapes and text handling.
// =============================================================================

use tree_sitter::Node;

use super::helpers::node_text;

/// Best-effort head name of a type expression. Returns the simple name
/// for `type_identifier` / `identifier` / `generic_type` (just the
/// `name` field, not its args), the dotted text for
/// `nested_type_identifier` / `member_expression`, the element-type
/// head for `array_type`, and an empty string for shapes whose head
/// can't be reduced to a single name (unions, intersections, mapped,
/// conditional, etc.).
/// The head type name of one tuple element node. A labeled element parses as a
/// `required_parameter`/`optional_parameter` whose `type` field is a
/// `type_annotation` (`: T`); a `rest_type`/`optional_type` wraps the type as a
/// child; an unlabeled element IS the type node.
pub(super) fn tuple_element_head(child: &Node, src: &[u8]) -> String {
    match child.kind() {
        "required_parameter" | "optional_parameter" => child
            .child_by_field_name("type")
            .map(|ta| type_annotation_head(&ta, src))
            .unwrap_or_default(),
        "optional_type" | "rest_type" => {
            for i in 0..child.child_count() {
                if let Some(n) = child.child(i) {
                    if n.is_named() {
                        return head_type_name(&n, src);
                    }
                }
            }
            String::new()
        }
        _ => head_type_name(child, src),
    }
}

/// The head type name inside a `type_annotation` (`: T` → `T`'s head).
pub(super) fn type_annotation_head(ta: &Node, src: &[u8]) -> String {
    for i in 0..ta.child_count() {
        if let Some(c) = ta.child(i) {
            if c.kind() != ":" {
                return head_type_name(&c, src);
            }
        }
    }
    String::new()
}

/// The text of a union/intersection branch, kept WHOLE so its type arguments
/// survive: `AndNot<TNonPromise>` stays applied instead of reducing to
/// `AndNot`. `intern_alias_target` interns this through `intern_type_str`,
/// which decomposes it into `Apply { base, args }` — every consumer keys on the
/// head, so the head readings are unchanged while the arguments become
/// available to substitution. Falls back to the head name for every other
/// shape, which carries no arguments to preserve.
pub(super) fn branch_type_text(node: &Node, src: &[u8]) -> String {
    match node.kind() {
        "generic_type" | "array_type" => node_text(*node, src),
        _ => head_type_name(node, src),
    }
}

pub(super) fn head_type_name(node: &Node, src: &[u8]) -> String {
    match node.kind() {
        "type_identifier" | "identifier" => node_text(*node, src),
        "nested_type_identifier" | "member_expression" => node_text(*node, src),
        "generic_type" => node
            .child_by_field_name("name")
            .map(|n| node_text(n, src))
            .unwrap_or_default(),
        "array_type" => {
            for i in 0..node.child_count() {
                let Some(child) = node.child(i) else { continue };
                if matches!(child.kind(), "[" | "]") {
                    continue;
                }
                let name = head_type_name(&child, src);
                if !name.is_empty() {
                    return name;
                }
            }
            String::new()
        }
        "parenthesized_type" | "readonly_type" => {
            for i in 0..node.child_count() {
                let Some(child) = node.child(i) else { continue };
                if matches!(child.kind(), "(" | ")" | "readonly") {
                    continue;
                }
                return head_type_name(&child, src);
            }
            String::new()
        }
        // `typeof value` — the value's name, so `ReturnType<typeof v>` carries `v`
        // as its single argument for the ReturnType intrinsic to resolve.
        "type_query" => {
            for i in 0..node.child_count() {
                let Some(child) = node.child(i) else { continue };
                if child.kind() == "typeof" {
                    continue;
                }
                let name = head_type_name(&child, src);
                if !name.is_empty() {
                    return name;
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}
