//! Configured source selection and literal-provider allocation are ingestion.
//! The resulting shared module graph traverses only numeric targets.
use super::*;
use crate::indexer::lexical::modules::scopes::Kind;

#[derive(Default)]
pub(super) struct Provider {
    parts: Vec<(String, SourceModuleId)>,
}

impl ModuleGraph {
    pub(in crate::indexer::resolve::engine) fn for_program(
        &self,
        program: super::super::program_graph::ProgramId,
        lookup: &dyn SymbolLookup,
    ) -> Option<Self> {
        self.programs.nominal_context(program)?;
        let mut graph = Self {
            configuration: self.configuration.clone(),
            ..Default::default()
        };
        let mut augmentations = Vec::new();
        for (path, source) in self.programs.sources(program) {
            let input = self.inputs.get(&path)?;
            let isolated = self.programs.isolated(source)?;
            for unit in &input.units {
                let Some(scope) = &unit.source_scope else {
                    continue;
                };
                if scope.kind != Kind::Literal {
                    continue;
                }
                if unit.parent.0 != 0 || !scope.complete || !scope.container_valid || !scope.ambient
                {
                    return None;
                }
                let name = unit.source_name.as_ref()?;
                if isolated {
                    augmentations.push((name.clone(), path.clone(), unit.id));
                } else {
                    // A relative ambient declaration cannot create a provider.
                    if module_paths::relative_base(&path, name).is_some() {
                        return None;
                    }
                    graph
                        .providers
                        .entry(name.clone())
                        .or_default()
                        .parts
                        .push((path.clone(), unit.id));
                }
            }
            graph.inputs.insert(path, input.clone());
        }
        graph.paths = graph.source_paths();
        for (name, path, unit) in augmentations {
            if let Some(provider) = graph.providers.get_mut(&name) {
                provider.parts.push((path, unit));
            } else {
                // An augmentation contributes to an existing selected source;
                // it must never manufacture the module it purports to augment.
                let target = link(
                    &graph.paths,
                    graph.inputs.get(&path)?,
                    &name,
                    lookup,
                    &FxHashMap::default(),
                    &FxHashMap::default(),
                )?;
                let (target_path, _) = graph.paths.iter().find(|(_, id)| **id == target)?;
                let group = graph
                    .augmentations
                    .entry(target_path.clone())
                    .or_insert_with(|| Provider {
                        parts: vec![(target_path.clone(), SourceModuleId(0))],
                    });
                group.parts.push((path, unit));
            }
        }
        for group in graph
            .providers
            .values_mut()
            .chain(graph.augmentations.values_mut())
        {
            group
                .parts
                .sort_by_key(|(path, unit)| (path.clone(), unit.0));
        }
        graph.rebuild(lookup);
        Some(graph)
    }

    pub(super) fn install_provider_groups(
        &mut self,
    ) -> (FxHashMap<String, ModuleId>, FxHashMap<ModuleId, ModuleId>) {
        let mut names = FxHashMap::default();
        let mut redirects = FxHashMap::default();
        for (name, provider, external) in self
            .providers
            .iter()
            .map(|(name, provider)| (name, provider, true))
            .chain(
                self.augmentations
                    .iter()
                    .map(|(path, provider)| (path, provider, false)),
            )
        {
            let id = ModuleId(self.modules.len());
            let mut module = Module {
                incomplete: provider.parts.is_empty(),
                ..Default::default()
            };
            for (path, unit) in &provider.parts {
                let target = self
                    .paths
                    .get(path)
                    .and_then(|root| self.units.get(&(*root, *unit)))
                    .copied();
                match target {
                    Some(target) => {
                        module.assignment_parts.push(target);
                        module.stars.extend(
                            [
                                ExportDomain::Value,
                                ExportDomain::Type,
                                ExportDomain::ValueQuery,
                            ]
                            .map(|domain| (Target::Namespace(target), domain)),
                        );
                    }
                    None => module.incomplete = true,
                }
            }
            // Conflicting declarations remain competing targets. No first-part
            // winner or implicit cross-file declaration merge is introduced.
            self.modules.push(module);
            if external {
                names.insert(name.clone(), id);
            } else if let Some(&root) = self.paths.get(name) {
                redirects.insert(root, id);
            }
        }
        (names, redirects)
    }
}

#[cfg(test)]
#[path = "program_modules_tests.rs"]
mod tests;
