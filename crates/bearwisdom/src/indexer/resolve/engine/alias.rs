// =============================================================================
// engine/alias — type-alias expansion to a concrete TypeId
//
// A chain step rooted at, or yielding, a type alias must see THROUGH the alias
// to its target before member lookup: `type UserMap = Map<string, User>` looked
// up for `.get` must find `get` on `Map`, not on the alias name. Roslyn treats
// aliases transparently — the alias resolves to the underlying type.
//
// The `Application` arm (`type Foo<T> = Bar<…>`) reduces structurally. A
// member-less alias of any other kind (conditional / typeof / a redirect to a
// single nominal type) is followed through its flattened RHS head, recorded as
// the alias's field type — so `type X<T> = … ? Positive<T> : Negative<T>` is
// transparent to member access. An alias that declares its own members (an
// object-literal alias) keeps those members instead of being followed.
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
        // The alias's target as a TypeId, computed as owned data so the lookup
        // borrow ends before `ty` is reassigned. An `Application` alias reduces
        // to `root<args…>`; any other alias kind is transparent through its
        // flattened RHS head when it carries no members of its own.
        let target = match lookup.alias_target(&head) {
            Some(AliasTarget::Application { root, args }) => application_target(arena, root, args),
            _ => match transparent_alias_target(lookup, arena, &head) {
                Some(t) => t,
                None => break,
            },
        };
        // A self-referential alias (`type T = … T …`) makes no structural
        // progress — stop rather than spin to the depth bound.
        if head_qname(arena, target).as_deref() == Some(head.as_str()) {
            break;
        }
        // Substitute the alias's own generic params with the application's args.
        let params = lookup
            .generic_params(&head)
            .map(|p| p.to_vec())
            .unwrap_or_default();
        let ty_args = apply_args(arena, ty);
        ty = if params.is_empty() || ty_args.is_empty() {
            target
        } else {
            let map: FxHashMap<String, TypeId> = params.into_iter().zip(ty_args).collect();
            arena.rebind_class_params(target, &map)
        };
    }
    ty
}

/// The `Application` alias target `root<args…>` as a TypeId.
fn application_target(arena: &TypeArena, root: &str, args: &[String]) -> TypeId {
    let base = arena.class(root);
    if args.is_empty() {
        base
    } else {
        let arg_ids = args.iter().map(|a| arena.intern_type_str(a)).collect();
        arena.intern(Type::Apply { base, args: arg_ids })
    }
}

/// The next hop for a transparent (non-`Application`) alias: its flattened RHS
/// head, recorded as the alias's field type. `None` — leaving the type
/// unchanged — when `head` is not a type alias, or when the alias declares its
/// own members (an object-literal alias `type T = { … }` / `type T = B & { … }`
/// *is* those members, so following it would drop them), or when no field type
/// is recorded.
fn transparent_alias_target(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    head: &str,
) -> Option<TypeId> {
    let (id, is_alias) = match lookup.by_qualified_name(head) {
        Some(s) => (s.id, s.kind == "type_alias"),
        None => return None,
    };
    if !is_alias || !lookup.members_of_id(id).is_empty() {
        return None;
    }
    if let Some(id) = lookup.field_type_id(head) {
        return Some(id);
    }
    lookup
        .field_type_name(head)
        .map(|s| arena.intern_type_str(s))
}

#[cfg(test)]
#[path = "alias_tests.rs"]
mod tests;
