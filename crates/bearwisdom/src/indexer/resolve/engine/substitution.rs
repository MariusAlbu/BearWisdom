// =============================================================================
// engine/substitution — receiver-driven generic substitution
//
// A member's declared type is written in the DECLARING type's vocabulary
// (`find(): T`); the chain walks in the RECEIVER's (`Repository<User>`). This
// module translates between them: it builds the `{parameter name → TypeId}`
// map the receiver's applied arguments impose and rewrites a yielded type
// through it, whether the member was declared on the receiver's own type or on
// a supertype reached through generic `extends` edges.
// =============================================================================

use rustc_hash::{FxHashMap, FxHashSet};

use crate::indexer::resolve::engine::contract::{Symbol, SymbolLookup};
use crate::type_checker::core::types::{TypeArena, TypeId};

use super::chain::{apply_args, head_qname};

/// Upper bound on supertype-chain climbing when composing generic arguments.
const MAX_SUPERTYPE_DEPTH: usize = 8;

/// The bindings a receiver's applied type arguments impose on its own type's
/// generic parameters: `Repository<User>` with params `[T]` yields `T → User`.
/// Empty when the receiver applies no arguments or its type declares no
/// parameters.
///
/// Prefers the receiver's declaration id to read the parameters directly;
/// falls back to its nominal head only when the receiver has no bound
/// declaration (external / ambient / string-parsed).
pub(crate) fn receiver_env(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    receiver: TypeId,
    recv_id: Option<i64>,
) -> FxHashMap<String, TypeId> {
    let args = apply_args(arena, receiver);
    if args.is_empty() {
        return FxHashMap::default();
    }
    let params: Vec<String> = match recv_id.and_then(|id| lookup.generic_params_of(id)) {
        Some(p) => p,
        None => match head_qname(arena, receiver) {
            Some(head) => lookup.generic_params(&head).unwrap_or_default(),
            None => return FxHashMap::default(),
        },
    };
    params.into_iter().zip(args).collect()
}

/// Substitute the receiver's applied type arguments for the declaring type's
/// generic parameters throughout `yielded`. `Repository<User>` receiver with
/// params `[T]` rebinds `Class("T")` → `User`; a non-generic receiver, or a
/// declaring type with no parameters, leaves `yielded` untouched.
pub(crate) fn substitute_through(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    yielded: TypeId,
    receiver: TypeId,
    recv_id: Option<i64>,
) -> TypeId {
    let map = receiver_env(lookup, arena, receiver, recv_id);
    if map.is_empty() {
        return yielded;
    }
    arena.rebind_class_params(yielded, &map)
}

/// Bind a generic SUPERTYPE's parameters from the `extends`/`implements` edges
/// when `member` was found on that supertype, not on the receiver itself:
/// `class Child extends Base<User>` + `Base.m: T` yields `T`, which the edge
/// args `[User]` bind to `User`. The args ride on the edges (`inherits_args`),
/// not on the receiver type, so `substitute_through` — which reads only the
/// receiver's own applied args — cannot supply them.
///
/// Composes across the whole climb, not one hop: each hop's edge args are
/// first rewritten through the map accumulated so far, so a parameter threaded
/// down the hierarchy (`Child<T> extends Mid<T>`, `Mid<U> extends Base<U>`)
/// arrives at the declaring type carrying the receiver's own argument. The
/// receiver's `{param → arg}` map seeds the walk, which is what makes that
/// threading resolvable at all.
///
/// No-op when the receiver has no nominal head, the member is declared on the
/// receiver type itself (then `substitute_through` already handles it), or no
/// path of generic edges reaches the declaring type.
pub(crate) fn substitute_supertype_args(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    member: &Symbol,
    yielded: TypeId,
    receiver: TypeId,
    recv_id: Option<i64>,
) -> TypeId {
    let Some(recv_head) = head_qname(arena, receiver) else {
        return yielded;
    };
    // The member's declaring type qname is its own qname minus the final segment.
    let Some((decl_head, _)) = member.qualified_name.rsplit_once('.') else {
        return yielded;
    };
    if decl_head == recv_head {
        return yielded;
    }
    let seed = receiver_env(lookup, arena, receiver, recv_id);
    let mut seen: FxHashSet<String> = FxHashSet::default();
    let Some(map) = compose_to_supertype(lookup, arena, &recv_head, decl_head, &seed, 0, &mut seen)
    else {
        return yielded;
    };
    if map.is_empty() {
        return yielded;
    }
    arena.rebind_class_params(yielded, &map)
}

/// Walk `head`'s supertype edges toward `target`, composing each hop's
/// `{parameter → argument}` map through the one accumulated so far. Returns
/// the map that binds `target`'s OWN parameters, or `None` when no edge path
/// reaches it within `MAX_SUPERTYPE_DEPTH`.
fn compose_to_supertype(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    head: &str,
    target: &str,
    acc: &FxHashMap<String, TypeId>,
    depth: usize,
    seen: &mut FxHashSet<String>,
) -> Option<FxHashMap<String, TypeId>> {
    if depth >= MAX_SUPERTYPE_DEPTH || !seen.insert(head.to_string()) {
        return None;
    }
    // Probe the direct edge first: an inheritance link the store keys by symbol
    // id is not enumerated by `parent_class_qnames`, but its args are still
    // recorded under the two heads.
    let direct = hop_env(lookup, arena, head, target, acc);
    if !direct.is_empty() {
        return Some(direct);
    }
    for parent in lookup.parent_class_qnames(head) {
        let hop = hop_env(lookup, arena, head, parent, acc);
        if parent == target {
            return Some(hop);
        }
        if let Some(deep) =
            compose_to_supertype(lookup, arena, parent, target, &hop, depth + 1, seen)
        {
            return Some(deep);
        }
    }
    None
}

/// The `{parameter → argument}` map one `child -> parent` edge imposes, with
/// each argument first rewritten through `acc` so a parameter the child
/// inherited (`class Child<T> extends Base<T>`) carries the concrete type the
/// receiver bound it to.
///
/// Prefers the interned-id form of the edge args; falls back to interning the
/// stored arg strings for id-less stores (incremental reload / test fixtures).
fn hop_env(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    child: &str,
    parent: &str,
    acc: &FxHashMap<String, TypeId>,
) -> FxHashMap<String, TypeId> {
    let id_slice = lookup.parent_class_arg_ids(child, parent);
    let arg_ids: Vec<TypeId> = if !id_slice.is_empty() {
        id_slice.to_vec()
    } else {
        lookup
            .parent_class_args(child, parent)
            .iter()
            .map(|a| arena.intern_type_str(a))
            .collect()
    };
    if arg_ids.is_empty() {
        return FxHashMap::default();
    }
    let params = lookup.generic_params(parent).unwrap_or_default();
    if params.is_empty() {
        return FxHashMap::default();
    }
    params
        .into_iter()
        .zip(arg_ids.iter().map(|&a| {
            if acc.is_empty() {
                a
            } else {
                arena.rebind_class_params(a, acc)
            }
        }))
        .collect()
}

#[cfg(test)]
#[path = "substitution_tests.rs"]
mod tests;
