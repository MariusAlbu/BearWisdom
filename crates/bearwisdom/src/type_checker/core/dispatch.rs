// =============================================================================
// type_checker/core/dispatch.rs — method selection across dispatch axes
//
// Single front door for "given a method name and a call shape, pick the
// implementation". Dispatches on `profile.dispatch_axis`:
//
//   - `Receiver`   — classic single dispatch. Delegates straight to
//                    `MembersIndex::lookup` against the receiver type's
//                    supertype graph. Covers Java / C# / TS / Python /
//                    Ruby / Go method dispatch.
//
//   - `MultiArg`   — multi-method dispatch (R S4, Clojure defmulti,
//                    Common Lisp defmethod, Julia). Filters the candidate
//                    set by argument-type compatibility; picks the most
//                    specific match. Phase 4 ships the structural skeleton
//                    + first-match strategy; Phase 7 R/Lisp migrations
//                    sharpen specificity scoring.
//
//   - `ReturnType` — return-type dispatch (Haskell typeclass instances,
//                    Rust trait-method specialization when the caller's
//                    return-position binding pins it). Filters candidates
//                    by `expected_return` compatibility. Phase 4 ships the
//                    skeleton + first-match strategy; Phase 8 Haskell
//                    migration drives the full algorithm.
//
// Spec: research/architecture/02-engine-internal-architecture.html § Layer 4
//       research/architecture/04-implementation-phases.html § Phase 4
// =============================================================================

use super::types::{PrimKind, TypeArena, TypeId};
use crate::indexer::resolve::engine::{SymbolInfo, SymbolLookup};
use crate::type_checker::core::members::MembersIndex;
use crate::type_checker::core::supertype::SupertypeGraph;
use crate::type_checker::core::symbol_types::SymbolTypeMap;
use crate::type_checker::profile::language_profile::{DispatchAxis, LanguageProfile};
use crate::type_checker::subtype::{is_assignable_to_typed_with, SubtypeResult};
use crate::types::EdgeKind;

/// Inputs for a dispatch query — all the information a candidate selector
/// could plausibly need across the three axes.
pub struct DispatchQuery<'a> {
    pub method_name: &'a str,
    pub receiver: TypeId,
    pub arg_types: &'a [TypeId],
    pub expected_return: Option<TypeId>,
    pub kind_filter: EdgeKind,
}

/// Select the method that should service this call. Returns the chosen
/// SymbolInfo, or None when no candidate matches.
pub fn select_method(
    query: &DispatchQuery,
    members: &MembersIndex,
    supertypes: &SupertypeGraph,
    symbol_types: &SymbolTypeMap,
    arena: &TypeArena,
    profile: &LanguageProfile,
    lookup: &dyn SymbolLookup,
) -> Option<SymbolInfo> {
    match profile.dispatch_axis {
        DispatchAxis::Receiver => select_receiver(query, members, supertypes, arena, profile),
        DispatchAxis::MultiArg => {
            select_multi_arg(query, members, supertypes, symbol_types, arena, profile, lookup)
        }
        DispatchAxis::ReturnType => {
            select_return_type(query, members, supertypes, symbol_types, arena, profile, lookup)
        }
    }
}

fn select_receiver(
    query: &DispatchQuery,
    members: &MembersIndex,
    supertypes: &SupertypeGraph,
    arena: &TypeArena,
    profile: &LanguageProfile,
) -> Option<SymbolInfo> {
    members.lookup(
        query.receiver,
        query.method_name,
        query.kind_filter,
        supertypes,
        arena,
        profile,
    )
}

/// Gather every candidate named `method_name` reachable on the receiver's
/// supertype chain, then keep those whose declared parameter types are
/// assignable from the call's `arg_types`. Pick the first surviving match
/// — a more refined scoring (most-specific) lives in the per-language
/// hook that the R / Lisp migrations will wire.
fn select_multi_arg(
    query: &DispatchQuery,
    members: &MembersIndex,
    supertypes: &SupertypeGraph,
    symbol_types: &SymbolTypeMap,
    arena: &TypeArena,
    profile: &LanguageProfile,
    lookup: &dyn SymbolLookup,
) -> Option<SymbolInfo> {
    for candidate in candidates(query, members, supertypes) {
        let Some(data) = symbol_types.get(candidate.id) else {
            // No type info → can't decide. Accept conservatively when the
            // call has zero args; otherwise skip.
            if query.arg_types.is_empty() {
                return Some(candidate);
            }
            continue;
        };
        if args_assignable(&data.param_types, query.arg_types, arena, lookup, profile.primitive_mapping) {
            return Some(candidate);
        }
    }
    None
}

fn select_return_type(
    query: &DispatchQuery,
    members: &MembersIndex,
    supertypes: &SupertypeGraph,
    symbol_types: &SymbolTypeMap,
    arena: &TypeArena,
    profile: &LanguageProfile,
    lookup: &dyn SymbolLookup,
) -> Option<SymbolInfo> {
    if let Some(expected) = query.expected_return {
        for candidate in candidates(query, members, supertypes) {
            let Some(data) = symbol_types.get(candidate.id) else {
                continue;
            };
            let Some(return_ty) = data.return_type else {
                continue;
            };
            if matches!(
                is_assignable_to_typed_with(return_ty, expected, arena, lookup, profile.primitive_mapping),
                SubtypeResult::Yes
            ) {
                return Some(candidate);
            }
        }
    }
    // Fall back to single-dispatch when no expected_return is given or no
    // candidate's return type matches it — surfacing SOME target beats
    // silently missing when the return-type signal is weak or absent.
    select_receiver(query, members, supertypes, arena, profile)
}

/// Walk the receiver's supertype chain and yield every candidate named
/// `method_name` (regardless of kind filter — multi-arg matching applies
/// the kind filter at the parameter check, not the receiver walk).
fn candidates<'a>(
    query: &'a DispatchQuery,
    members: &'a MembersIndex,
    supertypes: &'a SupertypeGraph,
) -> impl Iterator<Item = SymbolInfo> + 'a {
    supertypes
        .walk_up(query.receiver)
        .flat_map(move |t| {
            members
                .direct_of(t)
                .iter()
                .chain(members.extensions_of(t).iter())
                .filter(move |s| s.name == query.method_name)
                .cloned()
                .collect::<Vec<_>>()
        })
}

fn args_assignable(
    params: &[TypeId],
    args: &[TypeId],
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    prims: &[(&str, PrimKind)],
) -> bool {
    if params.len() != args.len() {
        return false;
    }
    for (param, arg) in params.iter().zip(args.iter()) {
        match is_assignable_to_typed_with(*arg, *param, arena, lookup, prims) {
            SubtypeResult::Yes => continue,
            SubtypeResult::No => return false,
            // Unknown is treated as "accept" — conservative for the
            // multi-method case where the engine doesn't know enough to
            // reject, and rejecting would cause every call to miss.
            SubtypeResult::Unknown => continue,
        }
    }
    true
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
