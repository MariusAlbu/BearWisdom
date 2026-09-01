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
pub(crate) fn nominal_head(arena: &TypeArena, sym: &Symbol) -> TypeId {
    if crate::indexer::resolve::engine::support::is_type_kind(&sym.kind) {
        arena.decl(&sym.qualified_name, sym.id)
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
