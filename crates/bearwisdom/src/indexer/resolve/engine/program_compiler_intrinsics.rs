//! Selected source declarations and effective configuration supply alias bodies.
use super::super::{
    module_type_inputs::Slot,
    program_types::{Input, Signature},
};
use super::{Lookup, SymbolLookup};
use crate::indexer::lexical::type_syntax::compiler_intrinsics::Role;
use crate::type_checker::core::types::{Intrinsic, Type, TypeArena, TypeId};

pub(super) fn materialize(
    signature: &Signature,
    input: &Input,
    lookup: &Lookup<'_>,
    arena: &TypeArena,
) -> Option<Vec<TypeId>> {
    if !matches!(signature.slot, Slot::Alias) {
        return None;
    }
    let (_, role) = input
        .compiler_intrinsics
        .iter()
        .find(|(row, _)| *row == signature.declaration)?;
    let body = lookup
        .symbol_by_id(signature.declaration)
        .and(lookup.view.compiler_intrinsics)
        .map(|policy| match role {
            Role::IteratorReturn => Type::Intrinsic(if policy.strict_iterator_return {
                Intrinsic::Undefined
            } else {
                Intrinsic::Any
            }),
        })
        .unwrap_or(Type::Unknown);
    Some(vec![arena.intern(body)])
}
