// =============================================================================
// engine/alias — type-alias expansion to a concrete TypeId
//
// A chain step rooted at, or yielding, a type alias must see THROUGH the alias
// to its target before member lookup: `type UserMap = Map<string, User>` looked
// up for `.get` must find `get` on `Map`, not on the alias name. Roslyn treats
// aliases transparently — the alias resolves to the underlying type.
//
// Only the `Application` arm (`type Foo<T> = Bar<…>`) is expanded so far; the
// other `AliasTarget` arms (union, conditional, typeof, …) decline and leave the
// type unchanged — a later step.
// =============================================================================

use rustc_hash::FxHashMap;

use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::types::AliasTarget;

use super::chain::{apply_args, head_qname};
use super::contract::SymbolLookup;

/// Upper bound on alias-of-alias chaining; guards against a cyclic alias.
const MAX_ALIAS_DEPTH: usize = 8;

/// Expand a type alias to its target, substituting the alias's own generic
/// parameters with the type's applied arguments: `type Box<T> = Container<T>`
/// applied as `Box<User>` → `Container<User>`. Recurses through an alias of an
/// alias, bounded. A type that is not a registered `Application` alias is
/// returned unchanged.
pub(crate) fn expand(mut ty: TypeId, lookup: &dyn SymbolLookup, arena: &TypeArena) -> TypeId {
    for _ in 0..MAX_ALIAS_DEPTH {
        let Some(head) = head_qname(arena, ty) else {
            break;
        };
        // Snapshot out of the lookup borrow so `ty` can be reassigned below.
        let snapshot = match lookup.alias_target(&head) {
            Some(AliasTarget::Application { root, args }) => Some((root.clone(), args.clone())),
            _ => None,
        };
        let Some((root, args)) = snapshot else {
            break;
        };
        let params = lookup
            .generic_params(&head)
            .map(|p| p.to_vec())
            .unwrap_or_default();
        // The alias target as a TypeId: `root<args…>`.
        let base = arena.class(&root);
        let target = if args.is_empty() {
            base
        } else {
            let arg_ids = args.iter().map(|a| arena.intern_type_str(a)).collect();
            arena.intern(Type::Apply { base, args: arg_ids })
        };
        // Substitute the alias's own generic params with the application's args.
        let ty_args = apply_args(arena, ty);
        ty = if params.is_empty() || ty_args.is_empty() {
            target
        } else {
            let map: FxHashMap<String, TypeId> =
                params.into_iter().zip(ty_args).collect();
            arena.rebind_class_params(target, &map)
        };
    }
    ty
}

#[cfg(test)]
#[path = "alias_tests.rs"]
mod tests;
