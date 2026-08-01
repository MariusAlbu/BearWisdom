// =============================================================================
// engine/composite_members — member resolution across a composite alias
//
// An alias written as `A & B` or `A | B` has no members of its own: the member
// lives on one of its branches. This module resolves a member across those
// branches with the receiver's own type arguments substituted in, so a branch
// written in the alias's vocabulary (`Wrapper<A>` inside
// `type Shape<A> = … & Wrapper<A>`) is walked as the concrete application the
// receiver named (`Wrapper<User>` for `Shape<User>`).
//
// Intersection is member-ADDITIVE — the first branch carrying the member wins.
// Union admits only members present on EVERY arm, so a single arm without it
// makes the access invalid.
// =============================================================================

use crate::indexer::resolve::engine::contract::{Symbol, SymbolLookup};
use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::types::AliasTargetIds;

use super::chain::{apply_args, expand_receiver, head_qname, lookup_member_on_bounded, Receiver};
use super::generics::substitute_env;
use super::substitution::receiver_env;

/// Re-root an alias branch on the declaration it resolved to, keeping the
/// branch's applied arguments: branch `Matchers<void, T>` resolved to the
/// declaration `pkg.Matchers` yields `Apply { pkg.Matchers, [void, T] }`. A
/// branch with no arguments yields the plain class.
///
/// The arguments are what a member's declared type is substituted through, so
/// dropping them is what leaves a branch member typed by an open parameter.
fn applied_as(arena: &TypeArena, branch: TypeId, qname: &str) -> TypeId {
    let base = arena.class(qname);
    let args = apply_args(arena, branch);
    if args.is_empty() {
        base
    } else {
        arena.intern(Type::Apply { base, args })
    }
}

/// The branch rewritten into the RECEIVER's vocabulary: the alias's own
/// parameters are replaced by the arguments the receiver applied, so
/// `Wrapper<A>` inside `Shape<A>` reached as `Shape<User>` becomes
/// `Wrapper<User>`. A non-generic receiver leaves the branch unchanged.
fn branch_in_receiver_terms(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    recv_ty: TypeId,
    branch: TypeId,
) -> TypeId {
    let env = receiver_env(lookup, arena, recv_ty, None);
    substitute_env(arena, branch, &env)
}

/// Resolve `member` on the named branches of an intersection alias. A branch is
/// the head name of an `&` member (`Mapped` for `Mapped<Q> & {…}`);
/// anonymous object branches contribute no name and are skipped (their members are
/// flattened onto the alias itself). Each branch name is resolved to its
/// declaration(s) by simple name, then the member walk recurses by symbol id so a
/// branch shared across packages stays distinct and the branch's own supertypes
/// climb. Returns the first branch that carries `member`. `None` when `head` is
/// not an intersection alias or no branch carries the member.
pub(crate) fn lookup_member_on_intersection(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    recv_ty: TypeId,
    head: &str,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
    depth: usize,
) -> Option<Symbol> {
    let branches = match lookup.alias_target(head)? {
        AliasTargetIds::Intersection(branches)
        | AliasTargetIds::IntersectionMapped { branches, .. } => branches.clone(),
        _ => return None,
    };
    for &raw_branch in &branches {
        let branch_id = branch_in_receiver_terms(lookup, arena, recv_ty, raw_branch);
        // A branch is a NOMINAL type reference, possibly applied
        // (`Class("Foo")` / `Apply{Foo,[T]}`); resolving it to its declaration
        // is by the head name (`types_by_name`), the same path every type
        // reference uses — multi-candidate, so an ambiguous branch tries each.
        let branch = head_qname(arena, branch_id).unwrap_or_default();
        if branch.is_empty() || branch == head {
            continue;
        }
        for cand in lookup.types_by_name(&branch).iter() {
            let recv = expand_receiver(
                Receiver::new(
                    applied_as(arena, branch_id, &cand.qualified_name),
                    cand.id,
                ),
                lookup,
                arena,
                None,
            );
            // A branch that resolves back to the intersection itself makes no
            // progress — skip rather than recurse to the depth bound.
            if head_qname(arena, recv.ty).as_deref() == Some(head) {
                continue;
            }
            if let Some(m) = lookup_member_on_bounded(lookup, arena, recv, member, accept, depth - 1)
            {
                return Some(m);
            }
        }
    }
    None
}

/// Resolve `member` on a UNION alias `A | B | …`. TS union member access is
/// valid only for members present on EVERY arm, so the member resolves on the
/// union iff every named branch carries it — the canonical shape is a tagged
/// result union whose arms all `extends` a common base that declares the member.
/// Each branch head is resolved to its declaration(s) and the member walk
/// recurses by symbol id (climbing the branch's supertypes); the first arm's
/// resolution is returned once every arm has agreed it carries the member.
/// `None` when `head` is not a union alias, a branch is unnameable (a
/// primitive/literal arm that cannot carry the member), or any arm lacks it.
pub(crate) fn lookup_member_on_union(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    recv_ty: TypeId,
    head: &str,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
    depth: usize,
) -> Option<Symbol> {
    let branches = match lookup.alias_target(head)? {
        AliasTargetIds::Union(branches) => branches.clone(),
        _ => return None,
    };
    if branches.is_empty() {
        return None;
    }
    let mut resolved: Option<Symbol> = None;
    for &raw_branch in &branches {
        let branch_id = branch_in_receiver_terms(lookup, arena, recv_ty, raw_branch);
        // A branch is a NOMINAL type reference, possibly applied; resolve it to
        // its declaration by the head name (`types_by_name`) — the same path
        // every type reference uses.
        let branch = head_qname(arena, branch_id).unwrap_or_default();
        if branch.is_empty() || branch == head {
            // A primitive/literal/self arm cannot carry the member; union access
            // requires it on every arm, so the access is invalid.
            return None;
        }
        // A branch scoped under an enclosing declaration (a nested function's
        // synthesized `{outer}.{inner}$Ret`) is a DOTTED qualified name — an
        // exact `by_qualified_name` lookup finds its one real declaration.
        // `types_by_name` indexes by SIMPLE name only, so it would search for a
        // symbol literally NAMED the whole dotted string and find nothing.
        // Fall back to the simple-name search for a bare (unqualified) branch.
        let exact = lookup.by_qualified_name(&branch);
        let fallback_set = if exact.is_none() {
            Some(lookup.types_by_name(&branch))
        } else {
            None
        };
        let candidates: Vec<&Symbol> = match exact {
            Some(s) => vec![s],
            None => fallback_set.iter().flat_map(|s| s.iter()).collect(),
        };
        let mut branch_hit: Option<Symbol> = None;
        for cand in candidates {
            let recv = expand_receiver(
                Receiver::new(
                    applied_as(arena, branch_id, &cand.qualified_name),
                    cand.id,
                ),
                lookup,
                arena,
                None,
            );
            // A branch that resolves back to the union itself makes no progress.
            if head_qname(arena, recv.ty).as_deref() == Some(head) {
                continue;
            }
            if let Some(m) = lookup_member_on_bounded(lookup, arena, recv, member, accept, depth - 1)
            {
                branch_hit = Some(m);
                break;
            }
        }
        match branch_hit {
            None => return None,
            Some(m) => {
                if resolved.is_none() {
                    resolved = Some(m);
                }
            }
        }
    }
    resolved
}

/// The branch of `recv_ty`'s composite alias that DECLARES `member`, rewritten
/// into the receiver's vocabulary and re-rooted on the declaring type:
/// `Shape<User>` whose branch is `Wrapper<A>` yields `Wrapper<User>` for a
/// member declared on `Wrapper`.
///
/// A member found on a branch must yield through THAT branch — the alias head
/// carries the alias's own parameters, so substituting a branch member's
/// declared type through the alias leaves it open. `None` when the receiver is
/// not a composite alias or no branch declares the member, in which case the
/// caller keeps the receiver it already has.
pub(crate) fn declaring_branch_receiver(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    recv_ty: TypeId,
    member: &Symbol,
) -> Option<TypeId> {
    let (declaring, _) = member.qualified_name.rsplit_once('.')?;
    let head = head_qname(arena, recv_ty)?;
    if declaring == head {
        return None;
    }
    let branches = match lookup.alias_target(&head)? {
        AliasTargetIds::Intersection(branches)
        | AliasTargetIds::IntersectionMapped { branches, .. }
        | AliasTargetIds::Union(branches) => branches.clone(),
        _ => return None,
    };
    for raw in branches {
        let branch = branch_in_receiver_terms(lookup, arena, recv_ty, raw);
        let Some(branch_head) = head_qname(arena, branch) else {
            continue;
        };
        if branch_head == declaring || declaring.ends_with(&format!(".{branch_head}")) {
            return Some(applied_as(arena, branch, declaring));
        }
    }
    None
}
