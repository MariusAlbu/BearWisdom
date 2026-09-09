//! Source-bound direct-base receivers. No inheritance spelling seeds are read.
use super::{
    contract::{RefContext, SymbolLookup},
    head_decl::head_decl_id,
    lexical_type_ids::TypeBinder,
    module_graph::ModuleGraph,
    module_type_inputs::{lower, Recipe},
};
use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::{
    indexer::{lexical::BindingId, symbol_ids::SymbolIds},
    types::{ParsedFile, SymbolKind},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) enum Head {
    Declaration(i64),
    Import(usize),
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Input {
    pub owner: i64,
    pub head: Head,
    pub args: Vec<Recipe>,
}

pub(super) fn capture(
    file: &ParsedFile,
    ids: &SymbolIds,
    lookup: &super::compilation::Compilation,
    arena: &TypeArena,
) -> Vec<Input> {
    let Some(graph) = &file.flow.lexical else {
        return Vec::new();
    };
    let binder = TypeBinder {
        graph,
        path: &file.path,
        ids,
        lookup,
        source: Some(lookup),
        arena,
    };
    graph
        .types
        .bases
        .iter()
        .filter_map(|(&slot, base)| {
            let owner = ids.row_id(&file.path, slot)?;
            let (head, args) = if let Some((binding, args)) = base {
                let head = capture_head(file, ids, *binding);
                (head, args.iter().map(|r| lower(r, &binder)).collect())
            } else {
                (Head::Unknown, Vec::new())
            };
            Some(Input { owner, head, args })
        })
        .collect()
}

pub(super) fn capture_head(file: &ParsedFile, ids: &SymbolIds, binding: BindingId) -> Head {
    let Some(graph) = &file.flow.lexical else {
        return Head::Unknown;
    };
    if graph.module.imports.contains_key(&binding) {
        return Head::Import(binding.0);
    }
    graph
        .symbol_slots
        .get(&binding)
        .copied()
        .flatten()
        .filter(|&slot| {
            file.symbols
                .get(slot)
                .is_some_and(|s| s.kind == SymbolKind::Class)
        })
        .and_then(|slot| ids.row_id(&file.path, slot))
        .map(Head::Declaration)
        .unwrap_or(Head::Unknown)
}

pub(super) fn bind(
    modules: &ModuleGraph,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
) -> Vec<(i64, TypeId)> {
    let mut result = Vec::new();
    for file in modules.inputs.values() {
        for input in &file.bases {
            if lookup.symbol_by_id(input.owner).is_none() {
                continue;
            }
            let target = match input.head {
                Head::Declaration(id) => Some(id),
                Head::Import(binding) => modules
                    .binding(&file.path, BindingId(binding), false)
                    .declaration(),
                Head::Unknown => None,
            }
            .map(|id| lookup.canonical_decl_id(id));
            let ty = target
                .filter(|&id| id != lookup.canonical_decl_id(input.owner))
                .and_then(|id| lookup.symbol_by_id(id))
                .filter(|s| s.kind == "class")
                .map(|symbol| {
                    let base = arena.decl(&symbol.qualified_name, symbol.id);
                    if input.args.is_empty() {
                        return base;
                    }
                    let import = |binding| {
                        modules
                            .binding(&file.path, binding, true)
                            .declaration()
                            .and_then(|id| lookup.symbol_by_id(id))
                            .map(|s| arena.decl(&s.qualified_name, lookup.canonical_decl_id(s.id)))
                    };
                    arena.intern(Type::Apply {
                        base,
                        args: input
                            .args
                            .iter()
                            .map(|r| {
                                r.materialize_with_context(
                                    arena,
                                    &import,
                                    &|_, _| None,
                                    Some(lookup),
                                )
                            })
                            .collect(),
                    })
                })
                .unwrap_or_else(|| arena.intern(Type::Unknown));
            result.push((lookup.canonical_decl_id(input.owner), ty));
        }
    }
    result
}

pub(super) fn root(
    context: &RefContext,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
) -> Option<super::chain::Receiver> {
    let owner = lookup.enclosing_type_id_of(context.source_symbol_id?)?;
    let ty = lookup
        .canonical_type_info(lookup.canonical_decl_id(owner))?
        .base_type_id?;
    if !lookup.accepts_type_context(arena, ty) {
        return None;
    }
    let id = head_decl_id(arena, ty)?;
    lookup.symbol_by_id(id)?;
    Some(super::chain::Receiver::new(ty, id))
}

#[cfg(test)]
#[path = "base_receiver_tests.rs"]
mod tests;
