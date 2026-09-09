//! Source-attested module entities and alias paths; no semantic text lookup.
use super::*;

pub(super) fn namespaces(
    units: &[(scopes::Unit, Node)],
    graph: &mut LexicalBindings,
    output: &mut ModuleSyntax,
) {
    for (unit, _) in units
        .iter()
        .filter(|(u, _)| u.kind == scopes::Kind::Namespace)
    {
        let Some(name) = unit.name.filter(|_| unit.complete && unit.container_valid) else {
            continue;
        };
        let scope = if unit.parent.0 == 0 {
            ScopeId(0)
        } else {
            let Some((parent, _)) = units.iter().find(|(parent, _)| parent.id == unit.parent)
            else {
                continue;
            };
            parent.scope
        };
        let value = graph.declare(scope, name, 0, None);
        let ty = graph
            .type_entries
            .get(&(scope, name))
            .copied()
            .unwrap_or_else(|| graph.declare_type(scope, name));
        for binding in [value, ty].into_iter().collect::<HashSet<_>>() {
            let import = Import {
                source: ImportSource::Entity {
                    binding,
                    namespace: unit.id,
                },
                type_only: binding != value,
                selectors: vec![],
            };
            if output.imports.insert(binding, import).is_some() {
                output.ambiguous_imports.insert(binding);
            }
            output.import_units.insert(binding, unit.parent);
        }
    }
}

fn install(
    node: Node,
    owner: (ScopeId, SourceModuleId),
    source: &[u8],
    graph: &mut LexicalBindings,
    output: &mut ModuleSyntax,
    target: ImportSource,
    type_only: bool,
) -> Option<BindingId> {
    let name = text(node, source)?;
    let name = graph.intern(&name);
    let binding = if type_only {
        graph.declare_type(owner.0, name)
    } else {
        graph.declare(owner.0, name, 0, None)
    };
    graph.type_entries.entry((owner.0, name)).or_insert(binding);
    graph.kinds.entry(binding).or_insert(SymbolKind::Variable);
    graph.declarations.insert(
        SourceSpan {
            start: node.start_byte() as u32,
            end: node.end_byte() as u32,
        },
        binding,
    );
    if output
        .imports
        .insert(
            binding,
            Import {
                source: target,
                type_only,
                selectors: vec![],
            },
        )
        .is_some()
    {
        output.ambiguous_imports.insert(binding);
    }
    output.import_units.insert(binding, owner.1);
    Some(binding)
}

pub(super) fn required(
    node: Node,
    clause: Node,
    owner: (ScopeId, SourceModuleId),
    source: &[u8],
    graph: &mut LexicalBindings,
    output: &mut ModuleSyntax,
) -> bool {
    let Some(module) = clause
        .child_by_field_name("source")
        .and_then(|n| text(n, source))
    else {
        return false;
    };
    let Some(local) = clause.named_child(0) else {
        return false;
    };
    install(
        local,
        owner,
        source,
        graph,
        output,
        ImportSource::Assignment(module),
        token(node, "type"),
    )
    .is_some()
}

pub(super) fn internal(
    root: Node,
    owner: (ScopeId, SourceModuleId),
    source: &[u8],
    forms: &ModuleForms,
    graph: &mut LexicalBindings,
    output: &mut ModuleSyntax,
) -> bool {
    let mut cursor = root.walk();
    let mut complete = true;
    let mut aliases = Vec::new();
    for node in root.named_children(&mut cursor) {
        let alias = if node.kind() == forms.import_alias {
            Some(node)
        } else if node.kind() == forms.export {
            node.child_by_field_name("declaration")
                .filter(|n| n.kind() == forms.import_alias)
        } else {
            None
        };
        let Some(alias) = alias else {
            continue;
        };
        let binding = alias.named_child(0).and_then(|local| {
            install(
                local,
                owner,
                source,
                graph,
                output,
                ImportSource::Binding(None),
                token(alias, "type"),
            )
        });
        match (binding, alias.named_child(1)) {
            (Some(binding), Some(value)) => aliases.push((binding, value)),
            _ => complete = false,
        }
    }
    // Declare every alias before binding its RHS: later aliases shadow outer names too.
    for (binding, value) in aliases {
        match target(value, source, forms, graph) {
            Some(ExportTarget::Alias {
                binding: base,
                selectors,
            }) => {
                let import = output.imports.get_mut(&binding).unwrap();
                import.source = ImportSource::Binding(base);
                import.selectors = selectors;
            }
            _ => complete = false,
        }
    }
    complete
}

pub(super) fn target(
    mut node: Node,
    source: &[u8],
    forms: &ModuleForms,
    graph: &LexicalBindings,
) -> Option<ExportTarget> {
    let mut selectors = Vec::new();
    while let Some(&(_, object, property, _)) = forms.selections.iter().find(|f| f.0 == node.kind())
    {
        selectors.push(text(node.child_by_field_name(property)?, source)?);
        node = node.child_by_field_name(object)?;
    }
    if !forms.identifier_names.contains(&node.kind()) {
        return None;
    }
    let name = text(node, source)?;
    let binding = graph
        .name_id(&name)
        .and_then(|id| graph.reference_binding_at(node.start_byte() as u32, id));
    selectors.reverse();
    Some(ExportTarget::Alias { binding, selectors })
}

#[cfg(test)]
#[path = "lexical_module_aliases_tests.rs"]
mod tests;
