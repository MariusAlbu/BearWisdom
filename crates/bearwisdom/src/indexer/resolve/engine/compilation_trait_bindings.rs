//! Canonical trait Self binders survive snapshot persistence with TypeInfo.
use super::*;
use crate::indexer::resolve::engine::{module_trait_inputs::Owner, trait_graph};

impl Compilation {
    pub(super) fn prepare_trait_sources(&mut self) {
        for input in self.modules.inputs.values() {
            for header in &input.traits.headers {
                let Owner::Declaration(id) = header.owner else {
                    continue;
                };
                if !self.by_id.contains_key(&id) {
                    continue;
                }
                let info = self.type_info_by_id.entry(id).or_default();
                if info.trait_self_param.is_none() {
                    info.trait_self_param = Some(self.arena.intern_generic(GenericParamData {
                        name: "Self".into(),
                        owner_symbol_index: 0,
                        bound: None,
                        kind: crate::type_checker::core::types::GenericParamKind::Type,
                    }));
                }
            }
        }
        self.modules.traits = trait_graph::Graph::build(&self.modules, self, &self.arena);
    }

    pub(in crate::indexer::resolve::engine) fn trait_graph(&self) -> &trait_graph::Graph {
        &self.modules.traits
    }
    pub(in crate::indexer::resolve::engine) fn trait_file(
        &self,
        path: &str,
    ) -> Option<&trait_graph::File> {
        self.modules
            .traits
            .files
            .get(&crate::indexer::resolve::engine::module_paths::normalize(
                path,
            ))
    }
}

#[cfg(test)]
#[path = "compilation_trait_bindings_tests.rs"]
mod tests;
