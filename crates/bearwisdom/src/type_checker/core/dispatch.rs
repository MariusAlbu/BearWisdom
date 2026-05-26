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

use super::types::{PrimKind, Type, TypeArena, TypeId};
use crate::indexer::resolve::engine::{SymbolInfo, SymbolLookup};
use crate::type_checker::core::members::MembersIndex;
use crate::type_checker::core::supertype::SupertypeGraph;
use crate::type_checker::core::symbol_types::SymbolTypeMap;
use crate::type_checker::profile::language_profile::{DispatchAxis, LanguageProfile};
use crate::type_checker::subtype::{args_assignable, is_assignable_to_typed_with, SubtypeResult};
use crate::types::{CallArg, EdgeKind};

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
    // No argument-type match — fall back to receiver dispatch so the call
    // still resolves to a single-dispatch target rather than missing.
    select_receiver(query, members, supertypes, arena, profile)
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

/// Resolve a call's argument expressions to their types, for overload
/// disambiguation. Literals mint the matching primitive; a bare identifier is
/// chased to its local declared type; anything the extractor couldn't pin down
/// yields `Unknown` (which the dispatch arms treat as "can't reject"). The
/// resulting types feed `DispatchQuery::arg_types`.
pub(crate) fn resolve_arg_types(
    call_args: &[CallArg],
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
) -> Vec<TypeId> {
    call_args
        .iter()
        .map(|a| resolve_arg_type(a, arena, lookup))
        .collect()
}

fn resolve_arg_type(arg: &CallArg, arena: &TypeArena, lookup: &dyn SymbolLookup) -> TypeId {
    match arg {
        // String-shaped literals all type as the string primitive.
        CallArg::StringLit(_) | CallArg::TemplateLit(_) | CallArg::TaggedTemplate { .. } => {
            arena.primitive(PrimKind::Str)
        }
        // Numeric / boolean literals. Other literal text (null/undefined,
        // collection literals) is left Unknown rather than guessed.
        CallArg::Literal(text) => match scalar_literal_kind(text) {
            Some(kind) => arena.primitive(kind),
            None => arena.intern(Type::Unknown),
        },
        // A bare identifier: chase its local declared type when known. The
        // name interns as a nominal `Class` — primitive disjointness against a
        // primitive-typed parameter is decided at compare time, not here.
        CallArg::Ident(name) => match lookup.local_type(name) {
            Some(ty) if !ty.is_empty() => arena.intern_type_str(&ty),
            _ => arena.intern(Type::Unknown),
        },
        CallArg::ObjectKeys(_) | CallArg::Other => arena.intern(Type::Unknown),
    }
}

/// Classify a literal's source text into a primitive kind. Returns None for
/// shapes the engine does not type as a scalar (null/undefined, collection
/// literals), leaving the argument Unknown.
fn scalar_literal_kind(text: &str) -> Option<PrimKind> {
    let t = text.trim();
    match t {
        "true" | "false" => Some(PrimKind::Bool),
        _ if t.parse::<i64>().is_ok() => Some(PrimKind::Int),
        _ if t.parse::<f64>().is_ok() => Some(PrimKind::Float),
        _ => None,
    }
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
