//! Source module evidence persists separately from runtime numeric graph nodes.
use super::*;
use crate::{indexer::lexical::modules::scopes::Kind, types::SourceSpan};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::indexer::resolve::engine) struct Evidence {
    pub kind: Kind,
    pub lexical_scope: usize,
    pub range: SourceSpan,
    pub body: SourceSpan,
    /// Source `declare` context, not an inference from the file extension.
    pub ambient: bool,
    #[serde(default)]
    pub container_valid: bool,
    /// Syntax surface capture only; program legality/merging is a separate proof.
    pub complete: bool,
}

pub(super) fn capture(
    file: &ParsedFile,
    graph: &LexicalBindings,
    ids: &SymbolIds,
    input: &mut ModuleInput,
) {
    let syntax = &graph.module;
    let names: std::collections::HashMap<_, _> = graph.interned_names().collect();
    input.source_complete = Some(syntax.complete);
    input.units = syntax
        .units
        .iter()
        .map(|unit| InputUnit {
            id: unit.id,
            parent: unit.parent,
            source_name: unit.name.map(|name| names[&name].to_owned()),
            source_scope: Some(Evidence {
                kind: unit.kind,
                lexical_scope: unit.scope.0,
                range: unit.range,
                body: unit.body,
                ambient: unit.ambient,
                container_valid: unit.container_valid,
                complete: unit.complete,
            }),
            exports: exports(&unit.exports, graph, &file.path, ids),
            assignments: assignments(&unit.assignments, graph, &file.path, ids),
            stars: unit
                .stars
                .iter()
                .flat_map(|(module, type_only)| {
                    [
                        ExportDomain::Value,
                        ExportDomain::Type,
                        ExportDomain::ValueQuery,
                    ]
                    .into_iter()
                    .filter(move |domain| !type_only || *domain != ExportDomain::Value)
                    .map(move |domain| {
                        (
                            InputTarget::Namespace {
                                module: module.clone(),
                            },
                            domain,
                        )
                    })
                })
                .collect(),
            wildcard_exclusions: input.wildcard_exclusions.clone(),
            ..Default::default()
        })
        .collect();
    input.source_spans = syntax
        .units
        .iter()
        .map(|unit| (unit.id, unit.range.start, unit.range.end))
        .collect();
    if let Some(root) = graph.scopes.first() {
        input
            .source_spans
            .push((SourceModuleId(0), root.start, root.end));
    }
    let positions: std::collections::HashMap<_, _> = input
        .units
        .iter()
        .enumerate()
        .map(|(index, unit)| (unit.id, index))
        .collect();
    let mut imports: Vec<_> = syntax.imports.iter().collect();
    imports.sort_unstable_by_key(|(binding, _)| binding.0);
    for (&binding, import) in imports {
        let owner = syntax
            .import_units
            .get(&binding)
            .copied()
            .unwrap_or_default();
        for domain in [
            ExportDomain::Value,
            ExportDomain::Type,
            ExportDomain::ValueQuery,
        ] {
            let target = if syntax.ambiguous_imports.contains(&binding) {
                InputTarget::Missing
            } else {
                imported(import, graph, domain, &file.path, ids)
            };
            let mut entry = InputBinding {
                binding: binding.0,
                domain,
                target,
            };
            if owner != SourceModuleId(0) {
                entry.target = if let Some(&position) = positions.get(&owner) {
                    input.units[position].imports.push(entry.clone());
                    InputTarget::Binding {
                        module: owner,
                        binding: binding.0,
                        domain,
                    }
                } else {
                    InputTarget::Missing
                };
            }
            // File environments query unique file-local BindingIds. The forwarding
            // edge preserves that API without pretending imports live at the root.
            input.imports.push(entry);
        }
    }
}

#[cfg(test)]
#[path = "module_scope_inputs_tests.rs"]
mod tests;
