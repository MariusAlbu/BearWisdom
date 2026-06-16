// =============================================================================
// engine/alias — type-alias expansion to a concrete TypeSymbol
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

use crate::types::AliasTarget;

use super::contract::SymbolLookup;
use super::type_symbol::TypeSymbol;

/// Upper bound on alias-of-alias chaining; guards against a cyclic alias.
const MAX_ALIAS_DEPTH: usize = 8;

/// Expand a type alias to its target, substituting the alias's own generic
/// parameters with the type's applied arguments: `type Box<T> = Container<T>`
/// applied as `Box<User>` → `Container<User>`. Recurses through an alias of an
/// alias, bounded. A type that is not a registered `Application` alias is
/// returned unchanged.
pub(crate) fn expand(mut ty: TypeSymbol, lookup: &dyn SymbolLookup) -> TypeSymbol {
    for _ in 0..MAX_ALIAS_DEPTH {
        // Snapshot out of the lookup borrow so `ty` can be reassigned below.
        let snapshot = match lookup.alias_target(&ty.qname) {
            Some(AliasTarget::Application { root, args }) => Some((root.clone(), args.clone())),
            _ => None,
        };
        let Some((root, args)) = snapshot else {
            break;
        };
        let params = lookup
            .generic_params(&ty.qname)
            .map(|p| p.to_vec())
            .unwrap_or_default();
        let target = TypeSymbol {
            qname: root,
            type_args: args.iter().map(|a| TypeSymbol::parse(a)).collect(),
        };
        ty = target.substitute(&params, &ty.type_args);
    }
    ty
}

#[cfg(test)]
#[path = "alias_tests.rs"]
mod tests;
