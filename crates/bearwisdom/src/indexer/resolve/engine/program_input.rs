//! Durable global declaration evidence; string decoding ends at graph lowering.
use crate::indexer::symbol_ids::SymbolIds;
use crate::types::{ParsedFile, SymbolKind};
use serde::{Deserialize, Serialize};

#[path = "program_member_inputs.rs"]
pub(super) mod members;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Part {
    #[serde(default)]
    pub unit: super::module_input::SourceModuleId,
    pub name: String,
    pub binding: Option<usize>,
    pub declaration: Option<i64>,
    pub kind: SymbolKind,
    pub type_space: bool,
    pub parameters: Vec<String>,
    pub plain_parameters: bool,
    pub members: Vec<String>,
    pub plain_merge: bool,
    #[serde(default)]
    pub plain_header: bool,
    #[serde(default)]
    pub type_heritage: bool,
    #[serde(default)]
    pub surface: Option<Vec<members::Input>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct Input {
    #[serde(default)]
    pub types: Option<super::program_types::Input>,
    pub isolated: bool,
    pub complete: bool,
    pub roots: Vec<Part>,
    pub augmentations: Vec<Part>,
    pub merge_rules: Vec<(SymbolKind, SymbolKind)>,
}

pub(super) fn capture(file: &ParsedFile, ids: &SymbolIds) -> Option<Input> {
    let graph = file.flow.lexical.as_ref()?;
    let source = graph.globals.as_ref()?;
    let names: std::collections::HashMap<_, _> = graph.interned_names().collect();
    let lower = |part: &crate::indexer::lexical::globals::Declaration| Part {
        unit: part.unit,
        name: names[&part.name].into(),
        binding: part.binding.map(|id| id.0),
        declaration: part.slot.and_then(|slot| ids.row_id(&file.path, slot)),
        kind: part.kind,
        type_space: part.type_space,
        parameters: part
            .parameters
            .iter()
            .map(|id| names[id].to_owned())
            .collect(),
        plain_parameters: part.plain_parameters,
        members: part.members.iter().map(|id| names[id].to_owned()).collect(),
        plain_merge: part.plain_merge,
        plain_header: part.plain_header,
        surface: part.surface.as_ref().map(|surface| {
            surface
                .iter()
                .map(|member| members::lower(member, &file.path, ids))
                .collect()
        }),
        type_heritage: part
            .slot
            .and_then(|slot| graph.types.interface_bases.get(&slot))
            .is_some_and(Option::is_some),
    };
    Some(Input {
        isolated: source.isolated,
        complete: source.complete,
        roots: source.roots.iter().map(lower).collect(),
        augmentations: source.augmentations.iter().map(lower).collect(),
        merge_rules: source.merge_rules.clone(),
        types: None,
    })
}

#[cfg(test)]
#[path = "program_input_tests.rs"]
mod tests;
