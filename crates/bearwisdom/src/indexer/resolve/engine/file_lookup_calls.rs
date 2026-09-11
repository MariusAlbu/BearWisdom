//! Overload selection at a file's call sites. A member call selects on the
//! receiver's configured surface; a chain-less call on an import binding
//! selects within the group that binding names.
use super::super::FileLookup;
use crate::indexer::resolve::engine::contract::flow_cache::OverloadCall;
use crate::indexer::resolve::engine::contract::FlowCacheLookup;
use crate::indexer::resolve::engine::program_view;
use crate::type_checker::core::types::TypeId;

pub(super) fn overloaded_call(
    lookup: &FileLookup<'_>,
    receiver: TypeId,
    selector: u32,
    actual: &[TypeId],
    explicit: &[TypeId],
) -> Option<Result<OverloadCall, ()>> {
    let program = lookup.program.as_ref()?;
    if lookup.source_private_member(selector).is_some() {
        return None;
    }
    let name = *lookup.method_names.get(&selector)?;
    let callbacks = lookup
        .lexical
        .as_ref()
        .and_then(|c| c.bindings.globals.as_ref())
        .map(|g| &g.calls.callbacks);
    program_view::overload_calls::contextual(
        program,
        receiver,
        name,
        actual,
        explicit,
        &|index| callbacks.is_some_and(|c| c.contains_key(&(selector, index))),
        &|index, context| {
            program_view::callback_bodies::infer(
                program,
                lookup,
                lookup.lexical.as_ref()?.bindings,
                callbacks?.get(&(selector, index))?,
                context,
            )
        },
        false,
    )
}

pub(super) fn overloaded_import_call(
    lookup: &FileLookup<'_>,
    site: u32,
    actual: &[TypeId],
    explicit: &[TypeId],
) -> Option<Result<OverloadCall, ()>> {
    let program = lookup.program.as_ref()?;
    let rows = lookup.lexical.as_ref()?.import_overloads_at(site)?;
    program_view::overload_calls::imports::select(program, rows, actual, explicit)
}
