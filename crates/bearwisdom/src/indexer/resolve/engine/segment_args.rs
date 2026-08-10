// =============================================================================
// engine/segment_args — call-site explicit type arguments
//
// A call segment can carry the type arguments written at the call site
// (`GetDependency<IOrgRepo>()`, `wrap::<Admin>(x)`). When the callee declares
// its own generic parameters, those arguments bind them positionally — the
// only binding source when a parameter appears solely in the return type.
// Also holds the head+args reattach for a root segment's split declared
// annotation.
// =============================================================================

use rustc_hash::FxHashMap;

use crate::indexer::resolve::engine::contract::{Symbol, SymbolLookup};
use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::types::ChainSegment;

/// Substitute a call segment's explicit type arguments into `yielded`.
///
/// Binds the callee's own generic parameters positionally from
/// `seg.type_arg_ids` (interning `seg.type_args` when no ids were hydrated)
/// and rewrites the yield through the bindings. Runs BEFORE argument-driven
/// inference so an explicit argument wins over an inferred one. A parameter
/// beyond the supplied arguments stays open for argument inference or the
/// unbound-param rewrite. Unchanged when the segment carries no arguments or
/// the callee declares no parameters.
pub(crate) fn bind_explicit_type_args(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    member: &Symbol,
    seg: &ChainSegment,
    yielded: TypeId,
) -> TypeId {
    if seg.type_args.is_empty() && seg.type_arg_ids.is_empty() {
        return yielded;
    }
    let Some(params) = lookup
        .generic_params_of(member.id)
        .or_else(|| lookup.generic_params(&member.qualified_name))
    else {
        return yielded;
    };
    if params.is_empty() {
        return yielded;
    }
    let mut map: FxHashMap<String, TypeId> = FxHashMap::default();
    for (i, param) in params.iter().enumerate() {
        let arg = seg
            .type_arg_ids
            .get(i)
            .copied()
            .or_else(|| seg.type_args.get(i).map(|a| arena.intern_type_str(a)));
        if let Some(ty) = arg {
            map.insert(param.clone(), ty);
        }
    }
    if map.is_empty() {
        return yielded;
    }
    crate::tracef!(
        "  EXPLICIT-ARGS '{}' binds {:?}",
        member.qualified_name,
        map.keys().collect::<Vec<_>>(),
    );
    arena.rebind_class_params(yielded, &map)
}

/// Attach a segment's in-source type arguments to a freshly-interned bare head
/// that didn't already carry its own. A declared annotation reaches the chain
/// pre-split — `repo: Repository<User>` as head `Repository` + args `[User]` —
/// so the args must be reattached, else generic substitution sees no arguments.
pub(crate) fn with_segment_args(arena: &TypeArena, id: TypeId, type_args: &[String]) -> TypeId {
    if type_args.is_empty() {
        return id;
    }
    match arena.get(id) {
        Type::Class(_) => {
            let args = type_args.iter().map(|a| arena.intern_type_str(a)).collect();
            arena.intern(Type::Apply { base: id, args })
        }
        // Already an application (or another structural type) — the inline args
        // win and the segment's split args are redundant.
        _ => id,
    }
}

#[cfg(test)]
#[path = "segment_args_tests.rs"]
mod tests;
