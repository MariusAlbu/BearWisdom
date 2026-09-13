// =============================================================================
// python/param_type_refs.rs — the annotated parameter type a bare-name
// initializer carries
//
// `def __init__(self, repo: Repo): self.repo = repo` states the member's type
// on the parameter: no constructor call names it, so the only type text at the
// declaration site is the annotation on the enclosing function's parameter of
// that name. A name the function rebinds is no longer that parameter, so the
// annotation is dropped rather than applied to whatever the rebinding holds.
// =============================================================================

use super::helpers::{extract_python_type_name, node_text};
use crate::types::{EdgeKind, ExtractedRef};
use tree_sitter::Node;

#[cfg(test)]
#[path = "param_type_refs_tests.rs"]
mod tests;

/// Emit the `TypeRef` a bare-identifier initializer takes from the annotated
/// parameter it names, attributing it to `symbol_index`. Emits nothing for any
/// other initializer shape, an unannotated parameter, or a name the function
/// rebinds.
pub(super) fn emit_for_identifier_rhs(
    rhs: &Node,
    source: &str,
    symbol_index: usize,
    refs: &mut Vec<ExtractedRef>,
) {
    if rhs.kind() != "identifier" {
        return;
    }
    let name = node_text(rhs, source);
    let Some(function) = enclosing_function(rhs) else {
        return;
    };
    if rebinds(&function, &name, source) {
        return;
    }
    let Some(type_name) = annotated_parameter_type(&function, &name, source) else {
        return;
    };
    refs.push(ExtractedRef {
        is_include: false,
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: symbol_index,
        target_name: type_name,
        kind: EdgeKind::TypeRef,
        line: rhs.start_position().row as u32,
        col: 0,
        module: None,
        chain: None,
        byte_offset: rhs.start_byte() as u32,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    });
}

/// The `def` whose parameter list can name `node`. A lambda declares
/// parameters without annotations, so a body inside one carries no type text.
fn enclosing_function<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(n) = current {
        match n.kind() {
            "function_definition" => return Some(n),
            "lambda" => return None,
            _ => current = n.parent(),
        }
    }
    None
}

/// The annotation text on `function`'s parameter named `name`.
fn annotated_parameter_type(function: &Node, name: &str, source: &str) -> Option<String> {
    let parameters = function.child_by_field_name("parameters")?;
    let mut cursor = parameters.walk();
    for child in parameters.children(&mut cursor) {
        if !matches!(child.kind(), "typed_parameter" | "typed_default_parameter") {
            continue;
        }
        let Some(type_node) = child.child_by_field_name("type") else {
            continue;
        };
        let declared = match child.kind() {
            "typed_parameter" => {
                let mut inner = child.walk();
                child
                    .children(&mut inner)
                    .find(|c| c.kind() == "identifier")
                    .map(|c| node_text(&c, source))
            }
            _ => child
                .child_by_field_name("name")
                .map(|c| node_text(&c, source)),
        };
        if declared.as_deref() != Some(name) {
            continue;
        }
        let type_name = extract_python_type_name(&type_node, source);
        return (!type_name.is_empty()).then_some(type_name);
    }
    None
}

/// Whether `function` binds `name` anywhere in its body: an assignment, a loop
/// target, a `with`/`except` alias, or a walrus. Any of them means the name at
/// the initializer is no longer the parameter.
fn rebinds(function: &Node, name: &str, source: &str) -> bool {
    let Some(body) = function.child_by_field_name("body") else {
        return false;
    };
    let mut stack = vec![body];
    while let Some(node) = stack.pop() {
        let binder = match node.kind() {
            "assignment" | "for_statement" => node.child_by_field_name("left"),
            "named_expression" => node.child_by_field_name("name"),
            "as_pattern" => node.child_by_field_name("alias"),
            _ => None,
        };
        if let Some(binder) = binder {
            if binds_name(&binder, name, source) {
                return true;
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    false
}

/// Whether a binding target names `name` — directly, or as one element of a
/// tuple / list unpacking.
fn binds_name(target: &Node, name: &str, source: &str) -> bool {
    match target.kind() {
        "identifier" => node_text(target, source) == name,
        "pattern_list" | "tuple_pattern" | "list_pattern" | "as_pattern_target" => {
            let mut cursor = target.walk();
            target
                .named_children(&mut cursor)
                .any(|c| binds_name(&c, name, source))
        }
        _ => false,
    }
}
