// =============================================================================
// ruby/instance_attrs.rs — the class members a method body's `@x = …`
// assignments declare
//
// Ruby states no field list: the assignment in a method body IS the
// declaration, and every other method of the class reads the same member. The
// declaration therefore belongs to the class, once per name however many
// methods assign it. The sigil stays part of the name — `@x` and the `x` an
// `attr_*` macro declares are two different members of the same class.
// =============================================================================

use super::helpers::{node_text, qualify, scope_from_prefix};
use crate::types::{ExtractedSymbol, SymbolKind, Visibility};
use tree_sitter::Node;

#[cfg(test)]
#[path = "instance_attrs_tests.rs"]
mod tests;

/// Assignment forms whose `left` field can name an instance variable.
const ASSIGNMENT_KINDS: &[&str] = &[
    "assignment",
    "operator_assignment",
    "command_assignment",
    "command_operator_assignment",
];

/// Declare every instance variable `method`'s body assigns as a property of
/// the declaration at `owner_index`, skipping names that declaration already
/// carries.
pub(super) fn declare_from_method(
    method: &Node,
    src: &[u8],
    symbols: &mut Vec<ExtractedSymbol>,
    owner_index: usize,
    owner_qname: &str,
) {
    let Some(body) = method.child_by_field_name("body") else {
        return;
    };
    for target in assigned_instance_variables(&body) {
        let name = node_text(&target, src);
        if name.is_empty() || declared_on(symbols.as_slice(), owner_index, &name) {
            continue;
        }
        symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: qualify(&name, owner_qname),
            kind: SymbolKind::Property,
            visibility: Some(Visibility::Private),
            start_line: target.start_position().row as u32,
            end_line: target.end_position().row as u32,
            start_col: target.start_position().column as u32,
            end_col: target.end_position().column as u32,
            signature: None,
            doc_comment: None,
            scope_path: scope_from_prefix(owner_qname),
            parent_index: Some(owner_index),
            byte_offset: target.start_byte() as u32,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        });
    }
}

/// Whether the declaration at `owner_index` already carries a member `name`.
fn declared_on(symbols: &[ExtractedSymbol], owner_index: usize, name: &str) -> bool {
    symbols.iter().any(|s| {
        s.parent_index == Some(owner_index)
            && s.name == name
            && matches!(s.kind, SymbolKind::Property | SymbolKind::Field)
    })
}

/// The instance-variable target of every assignment under `body`, in source
/// order. Nested blocks and conditionals assign the same object's members, so
/// the whole subtree counts.
fn assigned_instance_variables<'a>(body: &Node<'a>) -> Vec<Node<'a>> {
    let mut found = Vec::new();
    let mut stack = vec![*body];
    while let Some(node) = stack.pop() {
        if ASSIGNMENT_KINDS.contains(&node.kind()) {
            if let Some(left) = node.child_by_field_name("left") {
                if left.kind() == "instance_variable" {
                    found.push(left);
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    found.sort_by_key(|n| n.start_byte());
    found
}
