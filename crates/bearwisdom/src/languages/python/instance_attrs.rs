// =============================================================================
// python/instance_attrs.rs — the class member a `self.x = …` assignment declares
//
// An attribute assignment whose receiver is the instance is the declaration
// site of a member: the name is readable from every method of the class, so the
// symbol belongs to the class, not to the method whose body happens to run the
// assignment. One declaration is emitted per (class, name) — a later assignment
// to the same name answers to the one already declared.
// =============================================================================

use super::helpers::{detect_python_visibility, node_text};
use crate::types::{ExtractedSymbol, SymbolKind};
use tree_sitter::Node;

#[cfg(test)]
#[path = "instance_attrs_tests.rs"]
mod tests;

/// Whether `object` names the instance a method body runs on.
fn is_instance_receiver(object: &Node, source: &str) -> bool {
    object.kind() == "identifier" && matches!(node_text(object, source).as_str(), "self" | "cls")
}

/// The class whose members a declaration under `index` contributes to: the
/// first class on the parent chain. `None` for a function outside any class.
fn owning_class(mut index: usize, symbols: &[ExtractedSymbol]) -> Option<usize> {
    loop {
        if symbols[index].kind == SymbolKind::Class {
            return Some(index);
        }
        index = symbols[index].parent_index?;
    }
}

/// The index of the class property `self.<name> = …` declares, declaring it
/// when this is the first assignment to that name on the class. `None` when
/// the receiver is not the instance or no class encloses the assignment — the
/// caller then keeps the assignment's own binding.
pub(super) fn declare(
    assignment: &Node,
    left: &Node,
    name_node: &Node,
    name: &str,
    source: &str,
    symbols: &mut Vec<ExtractedSymbol>,
    parent_index: Option<usize>,
) -> Option<usize> {
    let object = left.child_by_field_name("object")?;
    if !is_instance_receiver(&object, source) {
        return None;
    }
    let class_index = owning_class(parent_index?, symbols.as_slice())?;
    // A value the class already carries under this name is the same member,
    // whether the class body stated it or an earlier method assigned it.
    if let Some(existing) = symbols.iter().position(|s| {
        s.parent_index == Some(class_index)
            && s.name == name
            && matches!(
                s.kind,
                SymbolKind::Property | SymbolKind::Field | SymbolKind::Variable
            )
    }) {
        return Some(existing);
    }
    let class_qname = symbols[class_index].qualified_name.clone();
    symbols.push(ExtractedSymbol {
        name: name.to_string(),
        qualified_name: format!("{class_qname}.{name}"),
        kind: SymbolKind::Property,
        visibility: detect_python_visibility(name),
        start_line: name_node.start_position().row as u32,
        end_line: assignment.end_position().row as u32,
        start_col: name_node.start_position().column as u32,
        end_col: assignment.end_position().column as u32,
        signature: None,
        doc_comment: None,
        scope_path: Some(class_qname),
        parent_index: Some(class_index),
        byte_offset: name_node.start_byte() as u32,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });
    Some(symbols.len() - 1)
}
