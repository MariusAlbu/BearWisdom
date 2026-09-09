//! Portable initializer and constructor inventories, lowered at source ingestion.
use super::super::program_input::members;
use super::{lower, source_signatures::SignatureId, Recipe, TypeBinder};
pub(in crate::indexer::resolve::engine) use crate::indexer::lexical::type_syntax::initializers::Expression;
use serde::{Deserialize, Serialize};
pub(in crate::indexer::resolve::engine) type Object =
    crate::indexer::lexical::type_syntax::initializers::objects::Input<Recipe, i64>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::indexer::resolve::engine) struct Input {
    pub declaration: Option<i64>,
    pub signature: SignatureId,
    pub target: Option<crate::types::SourceSpan>,
    pub expression: Expression<Recipe>,
    pub annotated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::indexer::resolve::engine) struct Class {
    pub owner: i64,
    pub abstract_: bool,
    pub has_base: bool,
    pub generics: Option<SignatureId>,
    pub surface: Option<Vec<members::Input>>,
}

pub(super) fn capture(binder: &TypeBinder) -> Vec<Input> {
    binder
        .graph
        .types
        .initializers
        .iter()
        .map(|input| Input {
            declaration: input
                .declaration
                .and_then(|slot| binder.ids.row_id(binder.path, slot)),
            signature: input.signature,
            target: input.target,
            expression: input.expression.map(&|ty| lower(ty, binder)),
            annotated: input.annotated,
        })
        .collect()
}

pub(super) fn classes(binder: &TypeBinder) -> Vec<Class> {
    binder
        .graph
        .globals
        .iter()
        .flat_map(|globals| &globals.classes)
        .filter_map(|(part, abstract_, has_base)| {
            let slot = part.slot?;
            Some(Class {
                owner: binder.ids.row_id(binder.path, slot)?,
                abstract_: *abstract_,
                has_base: *has_base,
                generics: binder
                    .graph
                    .types
                    .signatures
                    .iter()
                    .find(|signature| signature.declaration == Some(slot))
                    .map(|signature| signature.id),
                surface: part.surface.as_ref().map(|surface| {
                    surface
                        .iter()
                        .map(|member| members::lower(member, binder.path, binder.ids))
                        .collect()
                }),
            })
        })
        .collect()
}

pub(super) fn objects(binder: &TypeBinder) -> Vec<Object> {
    use crate::indexer::lexical::type_syntax::initializers::objects;
    binder
        .graph
        .types
        .objects
        .iter()
        .map(|input| Object {
            span: input.span,
            members: input.members.as_ref().map(|members| {
                members
                    .iter()
                    .map(|member| objects::Member {
                        name: member.name,
                        span: member.span,
                        kind: member.kind,
                        declaration: member
                            .declaration
                            .and_then(|slot| binder.ids.row_id(binder.path, slot)),
                        value: member.value.map(&|ty| lower(ty, binder)),
                    })
                    .collect()
            }),
        })
        .collect()
}

#[cfg(test)]
#[path = "program_initializer_inputs_tests.rs"]
mod tests;
