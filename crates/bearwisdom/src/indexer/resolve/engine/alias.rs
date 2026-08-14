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
use crate::types::AliasTargetIds;

use super::alias_gate::{head_names_nominal_type, uncontested_alias_target};
use super::chain::{apply_args, callable_named_return, head_qname};
use super::contract::SymbolLookup;

/// Upper bound on alias-of-alias chaining; guards against a cyclic alias.
const MAX_ALIAS_DEPTH: usize = 8;

/// Expand a type alias to its target, substituting the alias's own generic
/// parameters with the type's applied arguments: `type Box<T> = Container<T>`
/// applied as `Box<User>` → `Container<User>`. Recurses through an alias of an
/// alias, bounded. A type that is not a registered `Application` alias is
/// returned unchanged.
pub(crate) fn expand(ty: TypeId, lookup: &dyn SymbolLookup, arena: &TypeArena) -> TypeId {
    expand_with_id(ty, None, lookup, arena)
}

/// `expand` with the receiver's declaration id. The id resolves a bare-name alias
/// COLLISION (two `type Logger = …` in different files) to the alias the use site
/// imported — via `alias_target_by_id`, which the name-keyed map cannot
/// disambiguate (last writer wins). The id is consumed on the hop it resolves, so
/// later hops resolve by name as usual; `None` reproduces the plain name walk.
pub(crate) fn expand_with_id(
    mut ty: TypeId,
    mut recv_id: Option<i64>,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
) -> TypeId {
    for _ in 0..MAX_ALIAS_DEPTH {
        let Some(head) = head_qname(arena, ty) else {
            break;
        };
        // `NoInfer<T>` is a TypeScript intrinsic identity wrapper — it only blocks
        // inference and carries `T`'s members transparently. Unwrap it so a member
        // walk on a `NoInfer<T>`-typed receiver sees `T`.
        if head == "NoInfer" {
            if let Some(inner) = apply_args(arena, ty).first().copied() {
                ty = inner;
                continue;
            }
        }
        // A member-preserving / key-narrowing utility applied DIRECTLY (`Omit<T,K>`
        // / `Pick<T,K>` / `Partial<T>` / …, not via a named alias) carries the
        // wrapped type's members — unwrap to `T` like `NoInfer` so a walk on
        // `Omit<Base,K>` resolves on `Base`. This catches the form produced
        // mid-expansion (a decided conditional's branch, a method's return); the
        // aliased form (`type X = Omit<…>`) is handled in the match below.
        if is_member_preserving_utility(&head) {
            if let Some(inner) = apply_args(arena, ty).first().copied() {
                ty = inner;
                continue;
            }
        }
        // `ReturnType<F>` applied DIRECTLY resolves by the intrinsic's
        // semantics — the named callable's return — BEFORE the name-keyed
        // alias lookup, which a package's own `ReturnType` helper can shadow
        // (an unimporting use site means the lib intrinsic, and the name map
        // cannot know that). Same ordering the utility unwraps above get. An
        // uncaptured return stops rather than dereferencing a dead head.
        if head == "ReturnType" {
            if let [arg] = apply_args(arena, ty)[..] {
                let formatted = arena.format_type(arg);
                let arg_str = formatted.strip_prefix("typeof ").unwrap_or(&formatted).trim();
                // Only an UNAMBIGUOUS callee name resolves context-free —
                // several same-named callables need the use site's import
                // scope, which expansion does not carry. Stopping keeps the
                // head diagnosable instead of binding a foreign namesake.
                let callables = lookup
                    .by_name(arg_str)
                    .iter()
                    .filter(|s| matches!(s.kind.as_str(), "function" | "method"))
                    .take(2)
                    .count();
                if callables > 1 {
                    break;
                }
                match callable_named_return(lookup, arena, arg_str) {
                    Some(t) => {
                        ty = t;
                        continue;
                    }
                    None => break,
                }
            }
        }
        // The alias's target as a TypeId, computed as owned data so the lookup
        // borrow ends before `ty` is reassigned. An `Application` alias reduces
        // to `root<args…>`; any other alias kind is transparent through its
        // flattened RHS head when it carries no members of its own.
        // Id-keyed target wins on the head it describes — a bare-name alias
        // collision the use site disambiguates by the declaration it imported.
        // The name-keyed fallback is gated: a head contested by a nominal type
        // declaration keeps the nominal receiver (see `alias_gate`).
        // Consumed after this hop (later hops resolve by name); survives a prior
        // NoInfer/utility unwrap so `NoInfer<Logger>` still keys on `Logger`'s id.
        let by_id = recv_id.take().and_then(|id| lookup.alias_target_by_id(id));
        let target = match by_id.or_else(|| uncontested_alias_target(lookup, &head)) {
            // `ReturnType<typeof f>` / `ReturnType<F>` intrinsic — the return type
            // of the function value/type the single argument names. A captured
            // return is required; an uncaptured one (`break`) leaves the alias
            // unresolved rather than dereferencing a dead `ReturnType` class.
            Some(AliasTargetIds::Application { root, args })
                if head_qname(arena, *root).as_deref() == Some("ReturnType")
                    && args.len() == 1 =>
            {
                let arg_str = arena.format_type(args[0]);
                match callable_named_return(lookup, arena, &arg_str) {
                    Some(t) => t,
                    None => break,
                }
            }
            // Member-preserving / key-narrowing utility wrappers carry the wrapped
            // type's members: `Omit<T,K>` / `Pick<T,K>` narrow the key set, the
            // modifier utilities (`Partial` / `Required` / `Readonly` /
            // `NonNullable` / `Awaited`) keep every member. The utility itself is a
            // member-less intrinsic, so a walk on `Omit<Base,K>` finds nothing;
            // redirect to the wrapped type `T` (the conservative receiver for member
            // resolution) so the member resolves on it.
            Some(AliasTargetIds::Application { root, args })
                if head_qname(arena, *root)
                    .as_deref()
                    .is_some_and(is_member_preserving_utility)
                    && !args.is_empty() =>
            {
                args[0]
            }
            Some(AliasTargetIds::Application { root, args }) => {
                application_target(arena, *root, args)
            }
            // A union alias has no members of its own; reducing it transparently to
            // a single arm (via a recorded field type) drops the receiver's type
            // arguments. Stop here so the member walk resolves the member on each
            // arm WITH those args bound (`lookup_member_on_union` + `substitute_through`).
            Some(AliasTargetIds::Union(_)) => break,
            // A conditional `C extends E ? T : F`. Evaluate it only when binding the
            // alias's params to the application's args reduces `C extends E` to a
            // DECIDABLE literal comparison (`TDynamic extends true` with
            // `TDynamic=false` → the false branch). The `… => infer R` return-type
            // shape is resolved at the root (`resolve_return_type_extraction`), not
            // here; an undecidable guard keeps the prior transparent behaviour —
            // never a guessed branch on the TYPE. For MEMBER LOOKUP specifically, an
            // undecidable guard with no recorded field type carries BOTH branches as
            // an Intersection: not an assertion that the type IS both, just reuse of
            // intersection's first-arm-match member traversal so a member declared on
            // whichever branch actually applies (`MockedFunction<T>` vs `T`) is still
            // found, instead of leaving a member-less conditional head unresolved.
            Some(AliasTargetIds::Conditional {
                check,
                extends,
                true_branch,
                false_branch,
                infer_binding,
            }) if !arena.format_type(*extends).contains("=> infer ") => {
                let params = lookup.generic_params(&head).unwrap_or_default();
                let arg_ids = apply_args(arena, ty);
                // An `infer` capture decides the conditional structurally: if the
                // checked type IS an application of the extends head, the pattern
                // matched and the captured variable takes the argument it sits on.
                if let Some(captured) = infer_binding.as_ref().and_then(|(var, slot)| {
                    expand_infer_capture(
                        arena,
                        &params,
                        &arg_ids,
                        *check,
                        *extends,
                        *true_branch,
                        var,
                        *slot,
                    )
                }) {
                    captured
                } else {
                match decide_conditional(arena, &params, &arg_ids, *check, *extends) {
                    Some(true) => *true_branch,
                    Some(false) => *false_branch,
                    None => match transparent_alias_target(lookup, arena, &head) {
                        Some(t) => t,
                        None => arena.intern(Type::Intersection(vec![*true_branch, *false_branch])),
                    },
                }
                }
            }
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
        let params = lookup.generic_params(&head).unwrap_or_default();
        let ty_args = apply_args(arena, ty);
        ty = if params.is_empty() || ty_args.is_empty() {
            target
        } else {
            let map: FxHashMap<String, TypeId> = params.into_iter().zip(ty_args).collect();
            arena.rebind_class_params(target, &map)
        };
        crate::tracef!("  ALIAS '{}' -> {}", head, arena.format_type(ty));
    }
    ty
}

/// `true` when `root` is a TypeScript intrinsic utility whose result carries (a
/// subset of) the wrapped type's members, so a member walk sees THROUGH it to the
/// first type argument. `Omit` / `Pick` narrow the key set; the modifier
/// utilities keep every member. Same transparent treatment the `NoInfer`
/// intrinsic already gets, extended to the member-shape-preserving wrappers.
fn is_member_preserving_utility(root: &str) -> bool {
    matches!(
        root,
        "Omit" | "Pick" | "Partial" | "Required" | "Readonly" | "NonNullable" | "Awaited"
    )
}

/// Resolve a conditional whose `extends` clause captures with `infer`:
/// `type Elem<T> = T extends Array<infer U> ? U : never` applied as
/// `Elem<User[]>` yields `User`.
///
/// The check side is bound through the application's arguments first (`T` →
/// `Array<User>`), then the capture matches only when the bound check IS an
/// application of the extends head — the same head the pattern names — and
/// carries an argument at the captured slot. The true branch is rewritten
/// through that binding, so a true branch naming anything else (`Array<U>`,
/// `Wrapper<U>`) resolves too, not just a bare `U`.
///
/// `None` when the pattern does not match structurally: the caller keeps its
/// undecidable behaviour rather than picking a branch. Multi-capture clauses
/// never reach here — the extractor records only the single-capture case.
#[allow(clippy::too_many_arguments)]
fn expand_infer_capture(
    arena: &TypeArena,
    params: &[String],
    arg_ids: &[TypeId],
    check: TypeId,
    extends: TypeId,
    true_branch: TypeId,
    var: &str,
    slot: usize,
) -> Option<TypeId> {
    let bound_check = if params.is_empty() || arg_ids.is_empty() {
        check
    } else {
        let map: FxHashMap<String, TypeId> = params
            .iter()
            .cloned()
            .zip(arg_ids.iter().copied())
            .collect();
        arena.rebind_class_params(check, &map)
    };
    if head_qname(arena, bound_check)? != head_qname(arena, extends)? {
        return None;
    }
    let captured = *apply_args(arena, bound_check).get(slot)?;
    let mut env: FxHashMap<String, TypeId> = FxHashMap::default();
    env.insert(var.to_string(), captured);
    Some(arena.rebind_class_params(true_branch, &env))
}

/// Decide a conditional's `check extends extends_ty` after binding the alias's
/// generic params to the application's args. `Some(true/false)` only when both
/// sides reduce to concrete, comparable literals; `None` when undecidable — the
/// caller must not pick a branch then.
fn decide_conditional(
    arena: &TypeArena,
    params: &[String],
    arg_ids: &[TypeId],
    check: TypeId,
    extends: TypeId,
) -> Option<bool> {
    // A bare generic-param reference (`T`) binds to the application's matching
    // arg; any other type expression stays itself. Comparison is on nominal-head
    // TypeIds — no string rendering.
    let bind = |id: TypeId| -> TypeId {
        head_qname(arena, id)
            .and_then(|n| params.iter().position(|p| p.trim() == n.trim()))
            .and_then(|i| arg_ids.get(i).copied())
            .unwrap_or(id)
    };
    literal_extends(arena, bind(check), bind(extends))
}

/// Nominal-head TypeId of `id`: peels `Apply`/`Optional`/`AsyncWrapper`/
/// `Iterator` wrappers to the underlying base, so `Vec<User>` and `Vec<infer U>`
/// share the `Vec` head id (the args bind separately in the conditional guard).
fn head_type_id(arena: &TypeArena, id: TypeId) -> TypeId {
    let mut cur = id;
    loop {
        match arena.get(cur) {
            Type::Apply { base, .. } => cur = base,
            Type::Optional(inner) | Type::AsyncWrapper(inner) | Type::Iterator(inner) => {
                cur = inner
            }
            _ => return cur,
        }
    }
}

/// Minimal, SOUND subtype decision for the literal forms a captured conditional
/// guard uses. Identical types are assignable; `true`/`false` are distinct boolean
/// literals. Anything else is undecidable here — there is no full subtype lattice,
/// so `None` rather than a guess.
fn literal_extends(arena: &TypeArena, check: TypeId, extends: TypeId) -> Option<bool> {
    // Everything is assignable to a top type: `T extends any` / `T extends
    // unknown` is the distributive-conditional idiom and always takes the true
    // branch, whatever `T` bound to.
    if head_qname(arena, extends).as_deref().is_some_and(|n| n == "any" || n == "unknown") {
        return Some(true);
    }
    // Identical nominal heads are assignable (structural id equality).
    if head_type_id(arena, check) == head_type_id(arena, extends) {
        return Some(true);
    }
    // Distinct boolean-literal types are non-assignable. A bool literal interns
    // as `Class("true")`/`Class("false")` — its identity IS the value — so this is
    // a literal-value check, not a type-shape string.
    let is_bool_lit =
        |id: TypeId| head_qname(arena, id).as_deref().is_some_and(|n| n == "true" || n == "false");
    if is_bool_lit(check) && is_bool_lit(extends) {
        return Some(false);
    }
    None
}

/// The `Application` alias target `root<args…>` as a TypeId. Both `root` and
/// each element of `args` are already interned TypeIds from the Compilation map.
fn application_target(arena: &TypeArena, root: TypeId, args: &[TypeId]) -> TypeId {
    if args.is_empty() {
        root
    } else {
        arena.intern(Type::Apply { base: root, args: args.to_vec() })
    }
}

/// The next hop for a transparent (non-`Application`) alias: its flattened RHS
/// head, recorded as the alias's field type. `None` — leaving the type
/// unchanged — when no same-qname declaration is a type alias, when a nominal
/// type contests the qname (the scan covers ALL same-qname declarations, so
/// the refusal is independent of symbol insertion order), when the alias
/// declares its own members (an object-literal alias `type T = { … }` /
/// `type T = B & { … }` *is* those members, so following it would drop them),
/// or when no field type is recorded.
fn transparent_alias_target(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    head: &str,
) -> Option<TypeId> {
    if head_names_nominal_type(lookup, head) {
        return None;
    }
    let id = lookup
        .all_by_qualified_name(head)
        .iter()
        .find(|s| s.kind == "type_alias")?
        .id;
    if !lookup.members_of_id(id).is_empty() {
        return None;
    }
    if let Some(t) = lookup.field_type_id_of(id).or_else(|| lookup.field_type_id(head)) {
        return Some(t);
    }
    lookup
        .field_type_str(head)
        .map(|s| arena.intern_type_str(&s))
}

#[cfg(test)]
#[path = "alias_tests.rs"]
mod tests;
