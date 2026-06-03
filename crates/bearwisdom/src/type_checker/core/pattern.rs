// =============================================================================
// type_checker/core/pattern.rs — destructuring binding to TypeId
//
// Bind names introduced by a destructuring pattern to their inferred TypeId
// using the RHS's value type. Three pattern shapes cover the dominant
// languages:
//   - `Identifier(name)` — plain binding. Name → value type.
//   - `Object(Vec<(prop_name, sub_pattern)>)` — JS/TS/Python/Ruby object
//     destructuring. For each prop, look up the member on the value type
//     and recurse into the sub-pattern with the member's yielded type.
//   - `Array(Vec<sub_pattern>)` — JS/TS/Python/Rust array destructuring.
//     For each position, recurse with the value type's element type
//     (Iterator wrapper / Apply<List,[T]> first arg).
//   - `Rest(name)` — `...rest` collector. Binds to the value type as-is.
//
// Spec: research/architecture/02-engine-internal-architecture.html § Layer 4
//       research/architecture/04-implementation-phases.html § Phase 4
// =============================================================================

use super::types::{Type, TypeArena, TypeId};
use crate::type_checker::core::inference::unwrap_iterator;
use crate::type_checker::core::members::MembersIndex;
use crate::type_checker::core::supertype::SupertypeGraph;
use crate::type_checker::core::symbol_types::SymbolTypeMap;
use crate::type_checker::core::symbol_view::SymbolView;
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::EdgeKind;

/// The shape of a destructuring binding. Languages whose destructure syntax
/// goes beyond this set (Rust struct patterns with field renames, Python
/// star-unpacking inside tuple unpacking) compose these primitives in their
/// extractors and emit a synthetic Pattern tree.
#[derive(Debug, Clone)]
pub enum Pattern {
    /// `let x = rhs;` — binds `name` to the RHS's type unchanged.
    Identifier(String),
    /// `const { a, b: alias } = rhs;` — each `(prop, sub)` looks up `prop`
    /// on the RHS type and recurses into `sub` with the resolved member
    /// type. The sub-pattern is typically `Identifier(alias)` or
    /// `Identifier(prop)`.
    Object(Vec<(String, Pattern)>),
    /// `const [head, tail] = rhs;` — each entry recurses with the RHS's
    /// element type.
    Array(Vec<Pattern>),
    /// `const [...rest] = rhs;` / `const { ...rest } = rhs;` — binds the
    /// collector to the RHS type unchanged.
    Rest(String),
}

/// Bind a pattern against `value_ty` and produce the resulting (name, type)
/// pairs in encounter order.
///
/// Object property lookups consult `members.lookup` so the result respects
/// generic substitution + supertype walks. Array element extraction
/// consults `inference::unwrap_iterator` so the iteration peeling honours
/// the language profile's `iterator_method` axis.
pub fn bind(
    pattern: &Pattern,
    value_ty: TypeId,
    arena: &TypeArena,
    members: &MembersIndex,
    supertypes: &SupertypeGraph,
    symbol_types: &SymbolTypeMap,
    profile: &LanguageProfile,
) -> Vec<(String, TypeId)> {
    let mut out = Vec::new();
    bind_into(
        pattern,
        value_ty,
        arena,
        members,
        supertypes,
        symbol_types,
        profile,
        &mut out,
    );
    out
}

fn bind_into(
    pattern: &Pattern,
    value_ty: TypeId,
    arena: &TypeArena,
    members: &MembersIndex,
    supertypes: &SupertypeGraph,
    symbol_types: &SymbolTypeMap,
    profile: &LanguageProfile,
    out: &mut Vec<(String, TypeId)>,
) {
    match pattern {
        Pattern::Identifier(name) => {
            out.push((name.clone(), value_ty));
        }
        Pattern::Rest(name) => {
            out.push((name.clone(), value_ty));
        }
        Pattern::Object(props) => {
            for (prop, sub) in props {
                let prop_ty = lookup_member_type(
                    prop,
                    value_ty,
                    arena,
                    members,
                    supertypes,
                    symbol_types,
                    profile,
                );
                // If the property isn't found, bind to Type::Unknown so the
                // caller still has a deterministic name → type entry and
                // can record a miss. Skipping silently would leave the
                // binding undeclared downstream.
                let prop_ty = prop_ty.unwrap_or_else(|| arena.intern(Type::Unknown));
                bind_into(
                    sub,
                    prop_ty,
                    arena,
                    members,
                    supertypes,
                    symbol_types,
                    profile,
                    out,
                );
            }
        }
        Pattern::Array(elems) => {
            // Last element may be a Rest — it captures the rest of the
            // sequence and binds to the original value type, not the
            // element type. Everything else binds to the element type.
            let element_ty = unwrap_iterator(value_ty, arena, profile);
            let len = elems.len();
            for (idx, sub) in elems.iter().enumerate() {
                let ty = if matches!(sub, Pattern::Rest(_)) && idx == len - 1 {
                    value_ty
                } else {
                    element_ty
                };
                bind_into(
                    sub,
                    ty,
                    arena,
                    members,
                    supertypes,
                    symbol_types,
                    profile,
                    out,
                );
            }
        }
    }
}

/// Resolve `prop` against `value_ty` via `MembersIndex::lookup`, then
/// derive the member's declared / yielded TypeId from SymbolTypeMap.
fn lookup_member_type(
    prop: &str,
    value_ty: TypeId,
    arena: &TypeArena,
    members: &MembersIndex,
    supertypes: &SupertypeGraph,
    symbol_types: &SymbolTypeMap,
    profile: &LanguageProfile,
) -> Option<TypeId> {
    let sym = members.lookup(value_ty, prop, EdgeKind::TypeRef, supertypes, arena, profile)?;
    let view = SymbolView::new(&sym, symbol_types);
    view.declared_type().or(view.return_type())
}

#[cfg(test)]
#[path = "pattern_tests.rs"]
mod tests;
