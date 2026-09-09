//! Type-domain base syntax. Recognition never certifies semantic compatibility.
use super::{Capture, TypeExpr, TypeUses};
use tree_sitter::Node;

pub(super) fn capture(capture: &Capture, node: Node, uses: &mut TypeUses) {
    if !capture.syntax.type_bases.0.contains(&node.kind()) {
        return;
    }
    let Some(slot) = capture.slot(node) else {
        return;
    };
    uses.interface_bases.insert(slot, bases(capture, node));
}

fn bases(capture: &Capture, node: Node) -> Option<Vec<TypeExpr>> {
    if node.has_error() {
        return None;
    }
    if !supported_parameters(node) {
        return None;
    }
    let (_, clause, field) = capture.syntax.type_bases;
    let fields = ["name", "type_parameters", "body"].map(|name| node.child_by_field_name(name));
    let mut bases = Vec::new();
    let mut clauses = 0;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor).filter(|n| !n.is_extra()) {
        if fields.contains(&Some(child)) {
            continue;
        }
        if child.kind() != clause {
            return None;
        }
        clauses += 1;
        if clauses > 1 {
            return None;
        }
        let mut cursor = child.walk();
        let types: Vec<_> = child.children_by_field_name(field, &mut cursor).collect();
        let mut cursor = child.walk();
        if types.is_empty()
            || child
                .named_children(&mut cursor)
                .filter(|n| !n.is_extra())
                .any(|n| !types.contains(&n))
        {
            return None;
        }
        bases.extend(types.into_iter().map(|base| capture.expr(base)));
    }
    Some(bases)
}

fn supported_parameters(node: Node) -> bool {
    let Some(parameters) = node.child_by_field_name("type_parameters") else {
        return true;
    };
    let mut cursor = parameters.walk();
    for parameter in parameters
        .named_children(&mut cursor)
        .filter(|n| !n.is_extra())
    {
        let fields =
            ["name", "constraint", "value"].map(|field| parameter.child_by_field_name(field));
        if fields[0].is_none() {
            return false;
        }
        let mut cursor = parameter.walk();
        if parameter
            .children(&mut cursor)
            .filter(|n| !n.is_extra())
            .any(|n| !fields.contains(&Some(n)))
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
#[path = "lexical_type_heritage_tests.rs"]
mod tests;
