// =============================================================================
// engine/head_decl — the declaration a receiver's type head names
//
// A receiver carries a type plus, where one is in hand, the id of the
// declaration that type names. That id is what keeps two same-qname receiver
// types in different packages apart during the member walk. This module
// answers "which declaration does this head name" in three flavours:
// exact-qname, package-preferred, and the re-root a member-less value shim
// needs. (Import-scoped disambiguation of an ambiguous simple name lives with
// the walk — see `chain::import_scoped_decl_id`.)
//
// The answer is always a TYPE declaration. A value that happens to carry the
// head's qname — a property named `any` flattened out of an anonymous union
// arm, a same-named binding in another language's file — is not a receiver, and
// binding one makes every member on it miss.
// =============================================================================

use crate::indexer::resolve::engine::contract::{FileContext, Symbol, SymbolLookup};
use crate::indexer::resolve::engine::contract::util::is_type_like_kind;
use crate::indexer::resolve::engine::support::pick_ranked_candidate;
use crate::type_checker::core::types::{Type, TypeArena, TypeId};

use super::chain::{head_qname, Receiver};

/// The nominal head a bound symbol produces: a `Decl` for a member-bearing
/// TYPE declaration, a `Class` for anything else. A value binding's qname is
/// a name-shaped proxy for its type, and alias/namespace declarations stay
/// name-addressed so the alias-expansion machinery sees exactly the heads it
/// always has.
pub(crate) fn nominal_head(lookup: &dyn SymbolLookup, arena: &TypeArena, sym: &Symbol) -> TypeId {
    if crate::indexer::resolve::engine::support::is_type_kind(&sym.kind) {
        arena.decl(&sym.qualified_name, lookup.canonical_decl_id(sym.id))
    } else {
        arena.class(&sym.qualified_name)
    }
}

/// The declaration id a type's nominal head CARRIES — a `Decl` head, reached
/// through the same wrappers `head_qname` peels. `None` for a name-only head;
/// the recovery lookups below are the fallback for those.
pub(crate) fn head_decl_id(arena: &TypeArena, id: TypeId) -> Option<i64> {
    match arena.get(id) {
        Type::Decl { symbol_id, .. } => Some(symbol_id),
        Type::Apply { base, .. } => head_decl_id(arena, base),
        Type::Optional(inner) | Type::AsyncWrapper(inner) | Type::Iterator(inner) => {
            head_decl_id(arena, inner)
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "head_decl_tests.rs"]
mod tests;

/// The declaration `qname` names, accepted only when it is type-like. A head
/// resolves to a type; `by_qualified_name`'s first-winner may be a value
/// sharing the qname, so same-qname candidates are scanned for a type before
/// declining.
fn type_decl_by_qname<'a>(lookup: &'a dyn SymbolLookup, qname: &str) -> Option<&'a Symbol> {
    let hit = lookup.by_qualified_name(qname)?;
    if is_type_like_kind(&hit.kind) {
        return Some(hit);
    }
    let simple = qname.rsplit('.').next().unwrap_or(qname);
    lookup
        .by_name(simple)
        .into_iter()
        .find(|s| s.qualified_name == qname && is_type_like_kind(&s.kind))
}

/// The symbol id of the declaration a type's nominal head names: the exact-qname
/// declaration, or — for a simple-name head (an annotation root like `Page`) that
/// matches several type declarations across packages — the one the use site's
/// imports scope to. `None` for a head with no indexed declaration
/// (external/ambient/string-parsed) or when imports don't disambiguate; the walk
/// then falls back to qname-string member lookup.
pub(crate) fn head_symbol_id(
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    ty: TypeId,
    file_ctx: Option<&FileContext>,
) -> Option<i64> {
    // A head that carries its declaration needs no recovery.
    if let Some(id) = head_decl_id(arena, ty) {
        return Some(id);
    }
    let head = head_qname(arena, ty)?;
    if let Some(s) = type_decl_by_qname(lookup, &head) {
        return Some(s.id);
    }
    // A simple-name head names no exact qname but may match several type
    // declarations; bind the import-scoped one when the use site disambiguates
    // rather than leaving it id-less (where a later step would pick a first-winner).
    let fc = file_ctx?;
    let simple = head.rsplit('.').next().unwrap_or(&head);
    let types = lookup.types_by_name(simple);
    let candidates: Vec<&Symbol> = types.iter().collect();
    if candidates.len() < 2 {
        return None;
    }
    pick_ranked_candidate(fc, None, lookup, &candidates).map(|s| s.id)
}

/// Build a `Receiver` for the type a member yielded, pinning the declaration id
/// from the member's package context when possible. A same-qname type in the
/// member's package is preferred over the first-winner, so the chain stays
/// anchored to the package the use site established. Falls back to the plain
/// exact-qname declaration when no package id is recorded on the member or when
/// no same-package declaration exists (external/ambient heads).
pub(crate) fn yielded_receiver(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    ty: TypeId,
    member_package_id: Option<i64>,
) -> Receiver {
    let id = head_symbol_id_preferring_package(arena, lookup, ty, member_package_id);
    Receiver { ty, id }
}

/// The symbol id of the declaration a type's head names, preferring a
/// same-package declaration over the first-winner when `preferred_pkg` is known.
pub(crate) fn head_symbol_id_preferring_package(
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    ty: TypeId,
    preferred_pkg: Option<i64>,
) -> Option<i64> {
    // A head that carries its declaration needs no recovery.
    if let Some(id) = head_decl_id(arena, ty) {
        return Some(id);
    }
    let head = head_qname(arena, ty)?;
    let simple = head.rsplit('.').next().unwrap_or(&head);
    if let Some(pkg) = preferred_pkg {
        for s in lookup.by_name(simple) {
            if s.qualified_name == head
                && s.package_id == Some(pkg)
                && is_type_like_kind(&s.kind)
            {
                return Some(s.id);
            }
        }
    }
    type_decl_by_qname(lookup, &head).map(|s| s.id)
}

/// The type declaration a head should root on: the same-simple-name type whose
/// members the chain walks. `None` when no type shares the head's simple name —
/// the caller then tries value indirection.
///
/// When the head's OWN declaration is the type (a normal member-bearing type
/// name), that declaration is returned and the caller leaves the receiver
/// unchanged. When the head names a member-less value re-export shim and a
/// distinct same-simple-name type declares the members, that type is returned so
/// the caller re-roots onto it. A type candidate that carries members is
/// preferred over a member-less one, so the shim's own member-less value (which
/// can also be type-kind in some shapes) never shadows the real declaration.
pub(crate) fn receiver_type_for_head(
    lookup: &dyn SymbolLookup,
    head: &str,
    simple: &str,
) -> Option<Symbol> {
    let candidates = lookup.types_by_name(simple).to_vec();
    if candidates.is_empty() {
        return None;
    }
    // A type candidate whose qname equals the head and bears members is the head
    // itself as a member-bearing type — keep it (caller leaves the receiver as-is).
    if let Some(exact) = candidates
        .iter()
        .find(|c| c.qualified_name == head && type_has_members(lookup, c))
    {
        return Some(exact.clone());
    }
    // A package re-exporting a same-named type resolves to the RE-EXPORTED
    // declaration, not an arbitrary same-simple-name type — the re-export names
    // the authoritative source, keeping a foreign package's homonym out.
    if let Some(shell) = lookup.by_qualified_name(head) {
        for (orig, module) in lookup.reexports_from(&shell.file_path) {
            if orig.as_str() == simple {
                let want = format!("{module}.{simple}");
                if let Some(c) = candidates.iter().find(|c| c.qualified_name == want) {
                    return Some(c.clone());
                }
            }
        }
    }
    // Otherwise prefer a member-bearing type declaration (the interface that holds
    // the call signature / members), falling back to the first type candidate.
    candidates
        .iter()
        .find(|c| type_has_members(lookup, c))
        .or_else(|| candidates.first())
        .cloned()
}

/// `true` when the type declaration `sym` has at least one indexed member —
/// either id-keyed (`members_of_id`) or qname-keyed (`members_of`).
fn type_has_members(lookup: &dyn SymbolLookup, sym: &Symbol) -> bool {
    !lookup.members_of_id(sym.id).is_empty() || !lookup.members_of(&sym.qualified_name).is_empty()
}

/// Re-root a receiver whose nominal head is a BARE simple name onto its indexed
/// (package-prefixed) type declaration. A field/return annotation captures a type by
/// the name written in source (`theme: NbThemeService`), but the class is indexed
/// under its package-prefixed qname (`@nebular/theme.NbThemeService`) and its members
/// are keyed there — so a bare-headed receiver finds neither an id (no exact qname)
/// nor members (`members_of("NbThemeService")` is empty). Resolve the bare head to
/// its same-simple-name declaration, bind its id, and rewrite the head to the indexed
/// qname. Mirrors the bare-type-NAME re-rooting the static-access root does.
///
/// No-op when the head is already qualified, an exact-qname declaration exists, no
/// same-name type is indexed, or the pick is ambiguous.
pub(super) fn reroot_bare_head(
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    ty: TypeId,
    id: Option<i64>,
    file_ctx: Option<&FileContext>,
) -> Receiver {
    // A bound head IS its indexed declaration — nothing to re-root.
    if super::head_decl::head_decl_id(arena, ty).is_some() {
        return Receiver { ty, id };
    }
    let Some(head) = head_qname(arena, ty) else {
        return Receiver { ty, id };
    };
    if head.contains('.') || lookup.by_qualified_name(&head).is_some() {
        return Receiver { ty, id };
    }
    let types = lookup.types_by_name(&head);
    let cands: Vec<&Symbol> = types.iter().collect();
    let decl = match cands.as_slice() {
        [] => return Receiver { ty, id },
        [only] => *only,
        many => match file_ctx.and_then(|fc| pick_ranked_candidate(fc, None, lookup, many)) {
            Some(s) => s,
            // Several copies sharing ONE qname (a package re-exported through several
            // disk paths, e.g. `rxjs.Observable`) are an unambiguous re-root target
            // even when ranking can't separate them.
            None if many.iter().all(|c| c.qualified_name == many[0].qualified_name) => many[0],
            None => return Receiver { ty, id },
        },
    };
    if decl.qualified_name == head {
        return Receiver { ty, id: id.or(Some(decl.id)) };
    }
    Receiver {
        ty: rebind_head(arena, ty, &decl.qualified_name),
        id: Some(decl.id),
    }
}

/// Rewrite a type's nominal head `Class` to `qname`, preserving generic application
/// and the nullable/async/iterator wrappers `head_qname` looks through. A structural
/// type with no nominal head is returned unchanged.
/// Rewrite a type's `Class` head to the bound `Decl` form, preserving generic
/// application and the wrappers `head_qname` looks through — the Decl-side
/// mirror of `rebind_head`.
fn set_decl_head(arena: &TypeArena, ty: TypeId, qname: &str, symbol_id: i64) -> TypeId {
    match arena.get(ty) {
        Type::Class(_) => arena.decl(qname, symbol_id),
        Type::Apply { base, args } => {
            let base = set_decl_head(arena, base, qname, symbol_id);
            arena.intern(Type::Apply { base, args })
        }
        Type::Optional(inner) => {
            let i = set_decl_head(arena, inner, qname, symbol_id);
            arena.intern(Type::Optional(i))
        }
        Type::AsyncWrapper(inner) => {
            let i = set_decl_head(arena, inner, qname, symbol_id);
            arena.intern(Type::AsyncWrapper(i))
        }
        Type::Iterator(inner) => {
            let i = set_decl_head(arena, inner, qname, symbol_id);
            arena.intern(Type::Iterator(i))
        }
        _ => ty,
    }
}

/// Bind a stored type's head to its declaration when that binding is
/// UNAMBIGUOUS: the head qname names exactly one type-like declaration in the
/// whole index. An ambiguous or unknown head returns `None` and keeps its
/// `Class` form — read-time recovery (package preference, import ranking)
/// still owns those. Already-bound heads return `None` (nothing to do).
pub(super) fn bind_head_unique(
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    ty: TypeId,
) -> Option<TypeId> {
    if head_decl_id(arena, ty).is_some() {
        return None;
    }
    let head = head_qname(arena, ty)?;
    // Only QUALIFIED heads bind. A bare stored head is context-sensitive: it
    // may be the owning declaration's generic parameter (`find(): T`), which
    // must stay a `Class` for `rebind_class_params` to substitute — binding
    // it to a same-named declaration kills substitution for every generic
    // yield sharing the name. A dotted head is never a parameter.
    if !head.contains('.') && !head.contains("::") {
        return None;
    }
    let set = lookup.all_by_qualified_name(&head);
    let mut only: Option<&Symbol> = None;
    for s in set.iter() {
        if !is_type_like_kind(&s.kind) {
            continue;
        }
        if only.is_some_and(|prev| prev.id != s.id) {
            return None;
        }
        only = Some(s);
    }
    let decl = only?;
    Some(set_decl_head(
        arena,
        ty,
        &decl.qualified_name,
        lookup.canonical_decl_id(decl.id),
    ))
}

pub(super) fn rebind_head(arena: &TypeArena, ty: TypeId, qname: &str) -> TypeId {
    match arena.get(ty) {
        Type::Class(_) => arena.class(qname),
        Type::Apply { base, args } => {
            let base = rebind_head(arena, base, qname);
            arena.intern(Type::Apply { base, args })
        }
        Type::Optional(inner) => {
            let i = rebind_head(arena, inner, qname);
            arena.intern(Type::Optional(i))
        }
        Type::AsyncWrapper(inner) => {
            let i = rebind_head(arena, inner, qname);
            arena.intern(Type::AsyncWrapper(i))
        }
        Type::Iterator(inner) => {
            let i = rebind_head(arena, inner, qname);
            arena.intern(Type::Iterator(i))
        }
        _ => ty,
    }
}
