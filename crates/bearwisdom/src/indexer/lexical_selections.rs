//! Source selectors become detached bindings; no name-based semantic recovery.
use super::{
    modules::{Import, ModuleForms},
    Binding, BindingId, LexicalBindings,
};
use crate::types::SourceSpan;
use tree_sitter::Node;

pub(super) fn capture(root: Node, source: &[u8], forms: &ModuleForms, graph: &mut LexicalBindings) {
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if let Some(&(_, _, selector, type_space)) =
            forms.selections.iter().find(|f| f.0 == node.kind())
        {
            if let Some((root_binding, import)) = selection(node, source, forms, graph, type_space)
            {
                let binding = BindingId(graph.bindings.len());
                let scope = graph.bindings[root_binding.0].scope;
                graph.bindings.push(Binding {
                    scope,
                    available_from: 0,
                    annotation: None,
                });
                graph.module.imports.insert(binding, import);
                if let Some(&unit) = graph.module.import_units.get(&root_binding) {
                    graph.module.import_units.insert(binding, unit);
                }
                if graph.module.ambiguous_imports.contains(&root_binding) {
                    graph.module.ambiguous_imports.insert(binding);
                }
                if type_space {
                    graph.module.qualified_types.insert(
                        SourceSpan {
                            start: node.start_byte() as u32,
                            end: node.end_byte() as u32,
                        },
                        binding,
                    );
                } else if let Some(selector) = field_child(node, selector) {
                    graph
                        .module
                        .members
                        .insert(selector.start_byte() as u32, binding);
                }
            }
        }
        let mut cursor = node.walk();
        // Stable source order makes detached BindingIds deterministic across cache hydration.
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        pending.extend(children.into_iter().rev());
    }
}

fn selection(
    mut node: Node,
    source: &[u8],
    forms: &ModuleForms,
    graph: &LexicalBindings,
    type_space: bool,
) -> Option<(BindingId, Import)> {
    let mut selectors = Vec::new();
    while let Some(&(_, object, property, _)) = forms.selections.iter().find(|f| f.0 == node.kind())
    {
        selectors.push(
            field_child(node, property)?
                .utf8_text(source)
                .ok()?
                .to_owned(),
        );
        node = field_child(node, object)?;
    }
    let name = graph.name_id(node.utf8_text(source).ok()?)?;
    let binding = if type_space {
        graph.type_binding_at(node.start_byte() as u32, name)
    } else {
        graph.reference_binding_at(node.start_byte() as u32, name)
    }?;
    let mut import = graph.module.imports.get(&binding)?.clone();
    selectors.reverse();
    import.selectors.extend(selectors);
    Some((binding, import))
}

/// Read this visible node's field, not a nested hidden-rule inherited field.
/// Aliased recursive syntax can make child_by_field_name return a grandchild.
pub(super) fn field_child<'t>(node: Node<'t>, field: &str) -> Option<Node<'t>> {
    let mut cursor = node.walk();
    if !cursor.goto_first_child() {
        return None;
    }
    loop {
        if cursor.field_name() == Some(field) {
            return Some(cursor.node());
        }
        if !cursor.goto_next_sibling() {
            return None;
        }
    }
}

#[cfg(test)]
#[path = "lexical_selections_tests.rs"]
mod tests;
