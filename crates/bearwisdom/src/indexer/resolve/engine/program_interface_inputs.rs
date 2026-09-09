//! Source interface inventories include local/module scopes, not only globals.
use super::super::program_input::members;
use super::{lower, Recipe, TypeBinder};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::indexer::resolve::engine) struct Input {
    pub inherited_members: crate::indexer::lexical::globals::InheritedMembers,
    pub owner: i64,
    pub bases: Option<Vec<Recipe>>,
    pub plain_parameters: bool,
    pub generic_signature: Option<super::source_signatures::SignatureId>,
    pub surface: Option<Vec<members::Input>>,
}

pub(super) fn capture(binder: &TypeBinder) -> Vec<Input> {
    let Some(globals) = &binder.graph.globals else {
        return vec![];
    };
    globals
        .interfaces
        .iter()
        .filter_map(|part| {
            let slot = part.slot?;
            Some(Input {
                owner: binder.ids.row_id(binder.path, slot)?,
                inherited_members: globals.inherited_members,
                bases: binder
                    .graph
                    .types
                    .interface_bases
                    .get(&slot)
                    .and_then(|bases| bases.as_ref())
                    .map(|bases| bases.iter().map(|base| lower(base, binder)).collect()),
                plain_parameters: part.plain_parameters,
                generic_signature: binder
                    .graph
                    .types
                    .signatures
                    .iter()
                    .find(|signature| signature.declaration == Some(slot))
                    .map(|signature| signature.id),
                surface: part.surface.as_ref().map(|surface| {
                    surface
                        .iter()
                        .map(|m| members::lower(m, binder.path, binder.ids))
                        .collect()
                }),
            })
        })
        .collect()
}

/// Scoped interface candidates may add defaulted parameters. Preserve the full
/// binding group for private validation without relaxing workspace merge rules.
pub(super) fn groups(binder: &TypeBinder) -> Vec<Vec<i64>> {
    use crate::types::SymbolKind;
    let Some(globals) = &binder.graph.globals else {
        return vec![];
    };
    if !globals
        .merge_rules
        .contains(&(SymbolKind::Interface, SymbolKind::Interface))
    {
        return vec![];
    }
    let interfaces: rustc_hash::FxHashMap<_, _> = globals
        .interfaces
        .iter()
        .filter_map(|part| Some((part.slot?, part)))
        .collect();
    let mut groups: Vec<_> = binder
        .graph
        .type_symbol_slots
        .values()
        .filter(|slots| slots.len() > 1)
        .filter_map(|slots| {
            slots
                .iter()
                .map(|slot| {
                    let part = interfaces.get(slot)?;
                    if !part.plain_header
                        && !binder
                            .graph
                            .types
                            .interface_bases
                            .get(slot)
                            .is_some_and(Option::is_some)
                    {
                        return None;
                    }
                    binder.ids.row_id(binder.path, *slot)
                })
                .collect::<Option<Vec<_>>>()
        })
        .collect();
    groups.sort_unstable();
    groups
}

#[cfg(test)]
#[path = "program_interface_inputs_tests.rs"]
mod tests;
