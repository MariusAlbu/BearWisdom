// =============================================================================
// engine/module_input_lowering.rs — lexical export/import forms lowered to
// module-input targets
//
// Each captured export, assignment or import becomes an `InputTarget` per
// export domain, with local bindings traced to the declaration (or the
// entity: declaration plus namespace) they name.
// =============================================================================

use super::*;

pub(super) fn exports(
    exports: &[crate::indexer::lexical::modules::Export],
    graph: &LexicalBindings,
    path: &str,
    ids: &SymbolIds,
) -> Vec<InputExport> {
    let mut result = Vec::new();
    for export in exports {
        for domain in [
            ExportDomain::Value,
            ExportDomain::Type,
            ExportDomain::ValueQuery,
        ] {
            // An explicit type-only export shadows wildcard namesakes even
            // where it cannot supply a runtime value.
            let target = if export.type_only && domain == ExportDomain::Value {
                InputTarget::Missing
            } else {
                exported(&export.target, graph, domain, path, ids)
            };
            result.push(InputExport {
                name: export.name.clone(),
                domain,
                target,
            });
        }
    }
    result
}

fn exported(
    target: &ExportTarget,
    graph: &LexicalBindings,
    domain: ExportDomain,
    path: &str,
    ids: &SymbolIds,
) -> InputTarget {
    match target {
        ExportTarget::From(import) => imported(import, graph, domain, path, ids),
        ExportTarget::Module(unit) => InputTarget::LocalNamespace(*unit),
        ExportTarget::Local { value, ty } => {
            let binding = if domain == ExportDomain::Type {
                *ty
            } else if domain == ExportDomain::ValueQuery
                && ty.is_some_and(|id| graph.module.imports.contains_key(&id))
            {
                *ty
            } else {
                *value
            };
            local(graph, binding, domain, path, ids)
        }
        ExportTarget::Alias { binding, selectors } => InputTarget::Path {
            base: Box::new(local(graph, *binding, domain, path, ids)),
            names: selectors.clone(),
        },
        ExportTarget::Unknown => InputTarget::Missing,
    }
}

pub(super) fn assignments(
    targets: &[ExportTarget],
    graph: &LexicalBindings,
    path: &str,
    ids: &SymbolIds,
) -> Vec<(InputTarget, ExportDomain)> {
    targets
        .iter()
        .flat_map(|target| {
            [
                ExportDomain::Value,
                ExportDomain::Type,
                ExportDomain::ValueQuery,
            ]
            .map(|domain| (exported(target, graph, domain, path, ids), domain))
        })
        .collect()
}

pub(super) fn imported(
    import: &Import,
    graph: &LexicalBindings,
    domain: ExportDomain,
    path: &str,
    ids: &SymbolIds,
) -> InputTarget {
    use crate::indexer::lexical::modules::ImportSource;
    if import.type_only && domain == ExportDomain::Value {
        return InputTarget::Missing;
    }
    let base = match &import.source {
        ImportSource::Named { module, name } => InputTarget::From {
            module: module.clone(),
            name: name.clone(),
        },
        ImportSource::Namespace(module) => InputTarget::Namespace {
            module: module.clone(),
        },
        ImportSource::Assignment(module) => InputTarget::Assigned {
            module: module.clone(),
        },
        ImportSource::Binding(binding) => local(graph, *binding, domain, path, ids),
        ImportSource::Entity { binding, namespace } => InputTarget::Entity {
            declaration: Box::new(local_declaration(
                graph,
                *binding,
                domain == ExportDomain::Type,
                path,
                ids,
            )),
            namespace: Box::new(InputTarget::LocalNamespace(*namespace)),
        },
    };
    if import.selectors.is_empty() {
        base
    } else {
        InputTarget::Path {
            base: Box::new(base),
            names: import.selectors.clone(),
        }
    }
}

fn local(
    graph: &LexicalBindings,
    binding: Option<BindingId>,
    domain: ExportDomain,
    path: &str,
    ids: &SymbolIds,
) -> InputTarget {
    let Some(binding) = binding else {
        return InputTarget::Missing;
    };
    if graph.module.ambiguous_imports.contains(&binding) {
        return InputTarget::Missing;
    }
    if graph.module.imports.contains_key(&binding) {
        return InputTarget::Binding {
            module: graph
                .module
                .import_units
                .get(&binding)
                .copied()
                .unwrap_or_default(),
            binding: binding.0,
            domain,
        };
    }
    local_declaration(graph, binding, domain == ExportDomain::Type, path, ids)
}

fn local_declaration(
    graph: &LexicalBindings,
    binding: BindingId,
    type_space: bool,
    path: &str,
    ids: &SymbolIds,
) -> InputTarget {
    if type_space {
        if let Some(slots) = graph.type_symbol_slots.get(&binding) {
            return slots
                .iter()
                .map(|&slot| ids.row_id(path, slot))
                .collect::<Option<Vec<_>>>()
                .map(InputTarget::Declarations)
                .unwrap_or(InputTarget::Missing);
        }
    }
    if graph.symbol_slots.get(&binding) == Some(&None)
        && !graph.overload_bindings.contains(&binding)
    {
        return InputTarget::Incomplete;
    }
    if !type_space && graph.overload_bindings.contains(&binding) {
        let rows: Option<Vec<_>> = graph
            .symbols
            .iter()
            .filter(|(_, id)| **id == binding)
            .map(|(&slot, _)| ids.row_id(path, slot))
            .collect();
        if let Some(mut rows) = rows.filter(|rows| rows.len() > 1) {
            rows.sort_unstable();
            rows.dedup();
            return InputTarget::Overloads(rows);
        }
    }
    graph
        .symbol_slots
        .get(&binding)
        .copied()
        .flatten()
        .and_then(|slot| ids.row_id(path, slot))
        .map(InputTarget::Declaration)
        .unwrap_or(InputTarget::Missing)
}
