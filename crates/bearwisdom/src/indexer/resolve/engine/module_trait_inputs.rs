//! Durable ID-addressed trait contracts; these are evidence, not dispatch results.
use super::{
    lexical_type_ids::TypeBinder,
    module_type_inputs::{lower, Recipe},
};
use crate::indexer::{
    namespaces::{traits, SourceModuleId},
    symbol_ids::SymbolIds,
};
use crate::type_checker::core::types::{GenericParamKind, TypeArena};
use crate::types::{ParsedFile, SourceSpan};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Owner {
    Declaration(i64),
    Implementation(usize),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Header {
    pub owner: Owner,
    pub declaration: Option<i64>,
    pub self_binding: usize,
    pub unit: SourceModuleId,
    pub scope: usize,
    pub span: SourceSpan,
    pub members: Vec<i64>,
    pub enabled: bool,
    pub negative: bool,
    pub parameters: Vec<(usize, GenericParamKind)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Implementation {
    pub owner: usize,
    pub trait_type: Recipe,
    pub receiver: Recipe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Bound {
    pub owner: Owner,
    pub span: SourceSpan,
    pub subject: Recipe,
    pub traits: Vec<Recipe>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct QualifiedCall {
    pub root: SourceSpan,
    pub selector: u32,
    pub caller: Option<i64>,
    pub receiver: Recipe,
    pub trait_type: Recipe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Availability {
    pub scope: usize,
    pub parent: Option<usize>,
    pub bindings: Vec<usize>,
    pub complete: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct Data {
    #[serde(default)]
    pub providers: Vec<usize>,
    #[serde(default)]
    pub value_partners: Vec<(usize, usize)>,
    pub headers: Vec<Header>,
    pub implementations: Vec<Implementation>,
    pub bounds: Vec<Bound>,
    pub available: Vec<Availability>,
    pub method_scopes: Vec<(u32, usize)>,
    #[serde(default)]
    pub qualified_calls: Vec<QualifiedCall>,
}

pub(super) fn capture(
    file: &ParsedFile,
    ids: &SymbolIds,
    lookup: &super::compilation::Compilation,
    arena: &TypeArena,
) -> Data {
    let (Some(graph), Some(source)) = (&file.flow.lexical, &file.flow.namespaces) else {
        return Data::default();
    };
    let binder = TypeBinder {
        graph,
        path: &file.path,
        ids,
        lookup,
        source: Some(lookup),
        arena,
    };
    let providers = source
        .bindings
        .iter()
        .enumerate()
        .filter(|(_, b)| {
            b.targets
                .iter()
                .any(|t| !matches!(t, crate::indexer::namespaces::Target::Missing))
        })
        .map(|(id, _)| id)
        .collect();
    let value_partners = source.traits.value_partners.clone();
    let source = &source.traits;
    let owner = |owner| match owner {
        traits::Owner::Declaration(slot) => ids.row_id(&file.path, slot).map(Owner::Declaration),
        traits::Owner::Implementation(binding) => Some(Owner::Implementation(binding.0)),
    };
    let headers = source
        .headers
        .iter()
        .filter_map(|header| {
            let members: Vec<_> = header
                .members
                .iter()
                .filter_map(|&slot| ids.row_id(&file.path, slot))
                .collect();
            Some(Header {
                owner: owner(header.owner)?,
                declaration: header
                    .declaration
                    .and_then(|slot| ids.row_id(&file.path, slot)),
                self_binding: header.self_binding.0,
                unit: header.unit,
                scope: header.scope.0,
                span: header.span,
                enabled: header.enabled && members.len() == header.members.len(),
                negative: header.negative,
                members,
                parameters: header
                    .parameters
                    .iter()
                    .map(|&(id, kind)| (id.0, kind))
                    .collect(),
            })
        })
        .collect();
    let implementations = source
        .implementations
        .iter()
        .map(|item| Implementation {
            owner: item.owner.0,
            trait_type: lower(&item.trait_type, &binder),
            receiver: lower(&item.receiver, &binder),
        })
        .collect();
    let bounds = source
        .bounds
        .iter()
        .filter_map(|bound| {
            Some(Bound {
                owner: owner(bound.owner)?,
                span: bound.span,
                subject: lower(&bound.subject, &binder),
                traits: bound.traits.iter().map(|r| lower(r, &binder)).collect(),
            })
        })
        .collect();
    let mut available: Vec<_> = source
        .available
        .iter()
        .map(|(&scope, value)| Availability {
            scope: scope.0,
            parent: value.parent.map(|id| id.0),
            bindings: value.bindings.iter().map(|id| id.0).collect(),
            complete: value.complete,
        })
        .collect();
    available.sort_unstable_by_key(|a| a.scope);
    let mut method_scopes: Vec<_> = source
        .method_scopes
        .iter()
        .map(|(&byte, scope)| (byte, scope.0))
        .collect();
    method_scopes.sort_unstable();
    let mut qualified_calls: Vec<_> = source
        .qualified_calls
        .iter()
        .map(|(&selector, call)| QualifiedCall {
            root: call.root,
            selector,
            caller: call.caller.and_then(|slot| ids.row_id(&file.path, slot)),
            receiver: lower(&call.receiver, &binder),
            trait_type: lower(&call.trait_type, &binder),
        })
        .collect();
    qualified_calls.sort_unstable_by_key(|c| c.selector);
    Data {
        providers,
        value_partners,
        headers,
        implementations,
        bounds,
        available,
        method_scopes,
        qualified_calls,
    }
}

#[cfg(test)]
#[path = "module_trait_inputs_tests.rs"]
mod tests;
