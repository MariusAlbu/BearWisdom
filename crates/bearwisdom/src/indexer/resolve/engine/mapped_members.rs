// =============================================================================
// engine/mapped_members — member resolution through a mapped alias
//
// A mapped type (`{ [K in Src]: V }`) declares no members of its own: its keys
// come from the source it maps over and its member type from the value
// template. Resolving a member on one therefore asks two different questions —
// which type is the source (so a member declared THERE resolves), and does the
// source admit this key (so a member the mapping GENERATES resolves).
// =============================================================================

use rustc_hash::FxHashMap;

use crate::indexer::resolve::engine::contract::{Symbol, SymbolLookup};
use crate::type_checker::core::types::{TypeArena, TypeId};
use crate::types::AliasTargetIds;

use super::chain::{apply_args, head_qname, lookup_member, MAX_SUPERTYPE_DEPTH};

/// The source object type of a mapped alias `{ [K in keyof Src]: … }`, with the
/// mapped param bound to the receiver's applied type argument: `Override<A, B>`
/// (params `[A, B]`, source `A`) with receiver `Override<MutationObserverResult,
/// …>` yields `MutationObserverResult`. A source naming a concrete type rather
/// than a param is interned directly. `None` when `head` is not a mapped alias
/// or the mapped capture recorded no source.
pub(crate) fn mapped_source_type(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    ty: TypeId,
    head: &str,
) -> Option<TypeId> {
    let source_id = match lookup.alias_target(head) {
        Some(AliasTargetIds::Mapped { source, .. })
        | Some(AliasTargetIds::IntersectionMapped { source, .. })
            if !arena.format_type(*source).is_empty() =>
        {
            *source
        }
        _ => return None,
    };
    let source = arena.format_type(source_id);
    let args = apply_args(arena, ty);
    let params = lookup.generic_params(head).unwrap_or_default();
    if let Some(pos) = params.iter().position(|p| *p == source) {
        if let Some(&arg) = args.get(pos) {
            return Some(arg);
        }
        // Source names a parameter with NO applied argument — unbound. The bare
        // parameter name carries no members; its default (a `typeof <namespace>`)
        // is resolved by `lookup_member_via_unbound_mapped_source` through the
        // receiver's re-export closure instead.
        return None;
    }
    Some(source_id)
}

/// Resolve `member` on a mapped alias whose source parameter is UNBOUND — the
/// receiver supplied no type argument, so the parameter falls back to its
/// declared default. The shape: `{ [P in keyof Q]: Wrap<Q[P]> }` with
/// `Q extends Cons = typeof ns`. `keyof Q`'s keys are the members of the value
/// namespace `ns`, and the receiver type's package re-exports that whole
/// namespace (`export * from '@scope/pkg'`). Walk the receiver declaration's
/// wildcard re-exports and resolve `member` by qualified name in each re-exported
/// module.
///
/// Gated to: a Mapped / IntersectionMapped alias whose source is one of the
/// type's own generic parameters with NO applied argument (a bound source is
/// handled by `mapped_source_type`), declared in an EXTERNAL file (the namespace
/// + wholesale re-export shape this targets only occurs in library `.d.ts`).
/// `None` outside that shape or when no re-exported module carries the member.
pub(crate) fn lookup_member_via_unbound_mapped_source(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    ty: TypeId,
    head: &str,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
) -> Option<Symbol> {
    let source_id = match lookup.alias_target(head)? {
        AliasTargetIds::Mapped { source, .. }
        | AliasTargetIds::IntersectionMapped { source, .. }
            if !arena.format_type(*source).is_empty() =>
        {
            *source
        }
        _ => return None,
    };
    let source = arena.format_type(source_id);
    let params = lookup.generic_params(head).unwrap_or_default();
    let pos = params.iter().position(|p| *p == source)?;
    // A source param with a CONCRETE applied argument is bound — `mapped_source_type`
    // resolved it already. But a self-applied return type echoes its own parameters
    // as arguments (`f(): Mapped<Q, A, B>`), so the arg at `pos` is the parameter
    // name itself — still unbound, still defaulted. Treat an argument whose head is
    // one of the type's own parameters as unbound.
    let bound = apply_args(arena, ty)
        .get(pos)
        .and_then(|&a| head_qname(arena, a))
        .is_some_and(|h| !params.iter().any(|p| *p == h));
    if bound {
        return None;
    }
    let decl_file = lookup.by_qualified_name(head)?.file_path.clone();
    if !lookup.is_external_file(&decl_file) {
        return None;
    }
    for (orig, module) in lookup.reexports_from(&decl_file) {
        if orig.as_str() != "*" {
            continue;
        }
        let want = format!("{module}.{member}");
        for cand in lookup.by_name(member).iter() {
            if cand.qualified_name == want && accept(&cand.kind) {
                return Some(cand.clone());
            }
        }
    }
    None
}

/// Resolve `member` as a KEY the mapped alias `head` generates: a mapped type
/// over a union of string literals (`{ [K in 'click' | 'change']: V }`) admits
/// exactly those names, and every one of them carries the value template as its
/// type. The literal union is read from the source's own alias target, so this
/// is a structural question about the two aliases, not a table of names.
///
/// The generated key has no declaration of its own, so the resolution binds to
/// the mapped alias itself — the declaration that states the member exists, and
/// the definition a reader is sent to. `None` when `head` is not a mapped
/// alias, its source is not a literal union, or that union does not carry
/// `member`.
pub(crate) fn member_from_mapped_literal_key(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    head: &str,
    member: &str,
) -> Option<Symbol> {
    let source = match lookup.alias_target(head)? {
        AliasTargetIds::Mapped { source, .. }
        | AliasTargetIds::IntersectionMapped { source, .. } => *source,
        _ => return None,
    };
    let source_head = head_qname(arena, source)?;
    let branches = match lookup.alias_target(&source_head)? {
        AliasTargetIds::Union(branches) => branches.clone(),
        _ => return None,
    };
    let admits = branches.iter().any(|&b| {
        arena
            .format_type(b)
            .trim_matches(|c| c == '\'' || c == '"')
            == member
    });
    if !admits {
        return None;
    }
    lookup.by_qualified_name(head).cloned()
}
