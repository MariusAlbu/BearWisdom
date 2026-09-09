//! Callable source groups retain all origins; argument evidence selects a signature.
use super::*;
use crate::indexer::lexical::globals::member_surface::Kind;
use crate::indexer::resolve::engine::{
    contract::{
        flow_cache::{CallSignatureOrigin, OverloadCall},
        generic_return::substitute,
    },
    member_index::MemberNameId,
};
use crate::type_checker::core::types::TypeId;

#[path = "program_overload_order.rs"]
pub(super) mod ordering;

pub(in crate::indexer::resolve::engine) fn select(
    lookup: &Lookup,
    receiver: TypeId,
    name: MemberNameId,
    actual: &[TypeId],
    explicit: &[TypeId],
) -> Option<Result<OverloadCall, ()>> {
    contextual(
        lookup,
        receiver,
        name,
        actual,
        explicit,
        &|_| false,
        &|_, _| None,
        false,
    )
}

pub(in crate::indexer::resolve::engine) fn contextual(
    lookup: &Lookup,
    receiver: TypeId,
    name: MemberNameId,
    actual: &[TypeId],
    explicit: &[TypeId],
    deferred: &dyn Fn(usize) -> bool,
    callback: &dyn Fn(usize, TypeId) -> Option<TypeId>,
    single: bool,
) -> Option<Result<OverloadCall, ()>> {
    let arena = lookup.type_arena()?;
    let relation = super::merge_proof::types::Relation { lookup, arena };
    let receiver = relation.canonical(receiver, 0)?;
    let receiver = super::project_intrinsic(lookup, arena, receiver)?;
    if let Some(result) =
        super::object_members::call(lookup, receiver, name, actual, explicit, deferred, callback)
    {
        return Some(result);
    }
    let owner = crate::indexer::resolve::engine::head_decl::head_decl_id(arena, receiver)?;
    let surface = lookup.nominal_surface(owner)?;
    let members: Vec<_> = surface
        .members
        .iter()
        .filter(|m| m.origin.name == Some(name))
        .collect();
    if members.is_empty() || (!single && members.len() < 2) {
        return None;
    }
    Some((|| {
        if surface.incomplete || members.len() > 4096 {
            return Err(());
        }
        let order = ordering::candidates(lookup, &members);
        let env = crate::indexer::resolve::engine::bound_call::receiver_bindings(
            lookup,
            arena,
            receiver,
            Some(owner),
        );
        let parameters = &lookup
            .canonical_type_info(owner)
            .ok_or(())?
            .generic_param_ids;
        if parameters.iter().any(|p| !env.contains_key(p)) {
            return Err(());
        }
        let mut origins = Vec::new();
        let mut candidates = Vec::new();
        for member in members {
            if member.kind != Kind::Method || member.property.optional {
                return Err(());
            }
            let mut signature = member.signature.clone();
            let rewrite = |ty| substitute(arena, ty, &env);
            signature.parameters = signature.parameters.into_iter().map(rewrite).collect();
            signature.result = signature.result.map(rewrite);
            signature.constraints = signature
                .constraints
                .into_iter()
                .map(|ty| ty.map(rewrite))
                .collect();
            signature.defaults = signature
                .defaults
                .into_iter()
                .map(|ty| ty.map(rewrite))
                .collect();
            origins.push(CallSignatureOrigin {
                source: member.origin.source,
                span: member.origin.signature.0,
                declaration: member.origin.declaration,
            });
            candidates.push(signature);
        }
        let selection = super::call_selection::select(
            &relation,
            &candidates.iter().collect::<Vec<_>>(),
            order.as_deref(),
            actual,
            explicit,
            deferred,
            callback,
            false,
        )
        .ok_or(())?;
        let selected = selection.selected.ok_or(())?;
        let applied = selection.applied;
        Ok(OverloadCall {
            origins,
            selected,
            return_type: applied.result,
            parameters: applied.parameters,
        })
    })())
}

#[cfg(test)]
#[path = "program_overload_calls_tests.rs"]
mod tests;
