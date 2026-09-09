//! Durable recipes and selected-source generic arenas for rowless signatures.
use super::{lower, Recipe};
pub(in crate::indexer::resolve::engine) use crate::indexer::lexical::type_syntax::signatures::SignatureId;
use crate::indexer::lexical::type_syntax::signatures::{Generic, Signature};
use crate::indexer::resolve::engine::{
    contract::{SymbolLookup, TypeInfo},
    lexical_type_ids::TypeBinder,
};
use crate::type_checker::core::types::{
    GenericParamData, GenericParamId, GenericParamKind, TypeArena, TypeId,
};
use rustc_hash::FxHashMap;

pub(in crate::indexer::resolve::engine) type Input = Signature<Recipe, i64>;

pub(super) fn capture(binder: &TypeBinder) -> Vec<Input> {
    binder
        .graph
        .types
        .signatures
        .iter()
        .map(|s| Signature {
            id: s.id,
            declaration: s
                .declaration
                .and_then(|slot| binder.ids.row_id(binder.path, slot)),
            syntax: s.syntax.clone(),
            generics: s
                .generics
                .iter()
                .map(|p| Generic {
                    name: p.name,
                    constraint: p.constraint.as_ref().map(|r| lower(r, binder)),
                    default: p.default.as_ref().map(|r| lower(r, binder)),
                })
                .collect(),
            parameters: s.parameters.iter().map(|r| lower(r, binder)).collect(),
            result: s.result.as_ref().map(|r| lower(r, binder)),
        })
        .collect()
}

#[derive(Debug, Clone, Default)]
pub(in crate::indexer::resolve::engine) struct Bound {
    pub syntax: crate::indexer::lexical::globals::member_surface::Signature,
    pub generic_parameters: Vec<GenericParamId>,
    pub constraints: Vec<Option<TypeId>>,
    pub defaults: Vec<Option<TypeId>>,
    pub parameters: Vec<TypeId>,
    pub result: Option<TypeId>,
}

impl Bound {
    pub(in crate::indexer::resolve::engine) fn callable(
        &self,
        origin: crate::type_checker::core::types::CallableOrigin,
        arena: &TypeArena,
    ) -> Option<crate::type_checker::core::types::Callable<TypeId>> {
        use crate::type_checker::core::types::{Callable, CallableGeneric, CallableParameter};
        let count = self.generic_parameters.len();
        if !self.syntax.type_parameters_complete
            || self.syntax.type_parameters.len() != count
            || self.constraints.len() != count
            || self.defaults.len() != count
            || self.syntax.parameters.len() != self.parameters.len()
            || self.syntax.parameters.iter().any(|p| p.type_span.is_none())
        {
            return None;
        }
        Some(Callable {
            origin,
            complete: true,
            predicate: None,
            result: self.result?,
            generics: self
                .generic_parameters
                .iter()
                .enumerate()
                .map(|(i, &p)| CallableGeneric {
                    parameter: arena.generic_type(p),
                    constraint: self.constraints[i],
                    default: self.defaults[i],
                })
                .collect(),
            parameters: self
                .syntax
                .parameters
                .iter()
                .zip(&self.parameters)
                .map(|(p, &ty)| CallableParameter {
                    declaration: p.span,
                    ty,
                    optional: p.optional,
                    rest: p.rest,
                    receiver: p.receiver,
                })
                .collect(),
        })
    }
}

pub(in crate::indexer::resolve::engine) fn allocate(
    input: &super::Input,
    canonical: &FxHashMap<i64, i64>,
    info: &FxHashMap<i64, TypeInfo>,
    arena: &TypeArena,
) -> FxHashMap<SignatureId, Bound> {
    let names: FxHashMap<_, _> = input.names.iter().map(|(id, name)| (*id, name)).collect();
    input
        .source_signatures
        .iter()
        .map(|s| {
            let generic_parameters = match s.declaration {
                Some(row) => canonical
                    .get(&row)
                    .and_then(|owner| info.get(owner))
                    .filter(|info| info.generic_param_ids.len() >= s.generics.len())
                    .map(|info| info.generic_param_ids[..s.generics.len()].to_vec())
                    .unwrap_or_default(),
                None => s
                    .generics
                    .iter()
                    .map(|p| {
                        arena.intern_generic(GenericParamData {
                            name: p
                                .name
                                .and_then(|name| names.get(&name))
                                .map(|name| (*name).clone())
                                .unwrap_or_default(),
                            kind: GenericParamKind::Type,
                            owner_symbol_index: 0,
                            bound: None,
                        })
                    })
                    .collect(),
            };
            (
                s.id,
                Bound {
                    syntax: s.syntax.clone(),
                    generic_parameters,
                    ..Default::default()
                },
            )
        })
        .collect()
}

pub(in crate::indexer::resolve::engine) fn materialize(
    signature: &Input,
    bound: &mut Bound,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    path: &str,
) {
    materialize_with_query(signature, bound, lookup, arena, path, &|site| {
        lookup.source_value_type(site).flatten()
    });
}

pub(in crate::indexer::resolve::engine) fn materialize_with_query(
    signature: &Input,
    bound: &mut Bound,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    path: &str,
    query: &dyn Fn(crate::types::SourceSpan) -> Option<TypeId>,
) {
    let lower = |r: &Recipe| r.materialize_with_query(lookup, arena, path, query);
    bound.parameters = signature.parameters.iter().map(lower).collect();
    bound.result = signature.result.as_ref().map(lower);
    bound.constraints = signature
        .generics
        .iter()
        .map(|p| p.constraint.as_ref().map(lower))
        .collect();
    bound.defaults = signature
        .generics
        .iter()
        .map(|p| p.default.as_ref().map(lower))
        .collect();
}

#[cfg(test)]
#[path = "program_signature_types_tests.rs"]
mod tests;
