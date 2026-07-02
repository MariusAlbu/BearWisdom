// =============================================================================
// engine/chain — member-access binding (`a.b.c()`)
//
// Binds `a.b.c()` by rooting the first segment to a type, then walking
// member-by-member: find `b` on the root type (climbing supertypes), advance to
// the type `b` yields — substituting the receiver's type arguments through the
// member's declared type — find `c` there, and so on. The last segment's symbol
// is the resolution. The walk threads a canonical `TypeId` from the workspace
// arena, never a type string: `Repository<User>.find()` whose return is `T`
// advances to `User`. Type expressions enter as strings at the index boundary (a
// stored annotation, a forward-inferred local) and are interned ONCE into a
// TypeId at the root; every hop after is entity-based — arena substitution, not
// per-hop string parsing.
//
// Structural roots only for now (local-variable type, `this`/`self`, a declared
// annotation, a bare type name, an imported value's declared type); alias
// expansion is applied at each hop. Union dispatch is added measurement-driven.
// =============================================================================

use rustc_hash::FxHashMap;

use crate::indexer::resolve::engine::contract::{
    FileContext, RefContext, Symbol, SymbolInfo, SymbolLookup, RESOLVED_CONFIDENCE,
};
use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::types::{AliasTargetIds, SegmentKind};

use super::alias;
use super::cause::{Cause, CauseKind};
use super::support::{import_scoped_package_id, is_type_kind, pick_ranked_candidate};

/// Strategy tag for a member-chain bind produced by the new engine.
const STRATEGY: &str = "rule_chain";

/// Upper bound on supertype-chain climbing when locating an inherited member.
const MAX_SUPERTYPE_DEPTH: usize = 8;

/// Resolve a member chain to its final segment's symbol. `Err` when the root
/// can't be typed or a hop has no matching member — an honestly-unresolved
/// chain, carrying the first-uncaptured-type cause diagnosable from the
/// segment where the walk stopped (`None` when no death site could attribute
/// one from state already in hand).
pub fn bind_member_access(
    ref_ctx: &RefContext,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
) -> Result<SymbolInfo, Option<Cause>> {
    let chain = ref_ctx.extracted_ref.chain.as_ref().ok_or(None)?;
    // A single-segment "chain" carries no receiver to root; the bare ladder
    // handles it. Multi-segment only here.
    if chain.segments.len() < 2 {
        return Err(None);
    }
    // The walk is entity-based — it needs the workspace arena to intern roots
    // and substitute generics. The production lookup and the test double both
    // expose one; a lookup without an arena cannot type a chain.
    let arena = lookup.type_arena().ok_or(None)?;

    // The receiver threads a TypeId AND the symbol id of the declaration that
    // type names, when known. The id is the identity spine: a member step keys
    // on `members_of_id` so two declarations sharing a qname string stay
    // distinct, and the supertype climb is id-keyed. The id is `None` for a head
    // with no indexed declaration (external/ambient/string-parsed), and the walk
    // falls back to the qname-string member lookup there.
    let mut root = resolve_root(ref_ctx, file_ctx, lookup, arena, &chain.segments[0])?;
    // Pin the receiver's declaration id when the root is an untyped value typed by
    // a bare-name alias that COLLIDES (two `type Logger = …`): resolve the one the
    // use site imported so expansion keys on its id, not the name map's last
    // writer. Only the root needs it — yielded receivers carry qualified types.
    if root.id.is_none() {
        root.id = import_scoped_decl_id(ref_ctx, file_ctx, lookup, arena, root.ty);
    }
    let mut current = expand_receiver(root, lookup, arena, Some(file_ctx));
    let last = chain.segments.len() - 1;

    for (i, seg) in chain.segments.iter().enumerate().skip(1) {
        // Positional tuple access from an array-destructure binding
        // (`const [a, b] = x`, emitted as a `tuple_index:N` ComputedAccess
        // segment): select the receiver tuple's element N, not a named member.
        if let Some(idx) = tuple_index_of(seg) {
            let elem_ty = tuple_element_type(lookup, arena, current.ty, idx).ok_or(None)?;
            let elem_recv = expand_receiver(
                yielded_receiver(lookup, arena, elem_ty, None),
                lookup,
                arena,
                Some(file_ctx),
            );
            if i == last {
                return Ok(SymbolInfo {
                    target_symbol_id: elem_recv.id.ok_or(None)?,
                    confidence: RESOLVED_CONFIDENCE,
                    strategy: STRATEGY,
                    resolved_yield_type: Some(elem_recv.ty),
                    flow_emit: None,
                });
            }
            current = elem_recv;
            continue;
        }
        // `arr[i]` subscript on an array receiver projects the ELEMENT type — array
        // applications are homogeneous, so the index value is immaterial (covers
        // `arr[0]`, `arr[i]`, `arr[index]`). A non-array subscript (`obj['key']`)
        // yields None here and falls through to the named-member lookup below, where
        // the quoted key names a member. The `tuple_index:N` destructure form was
        // already consumed by the block above.
        if matches!(seg.kind, SegmentKind::ComputedAccess) {
            if let Some(elem_ty) = array_element_type(arena, current.ty) {
                let elem_recv = expand_receiver(
                    yielded_receiver(lookup, arena, elem_ty, None),
                    lookup,
                    arena,
                    Some(file_ctx),
                );
                if i == last {
                    return Ok(SymbolInfo {
                        target_symbol_id: elem_recv.id.ok_or(None)?,
                        confidence: RESOLVED_CONFIDENCE,
                        strategy: STRATEGY,
                        resolved_yield_type: Some(elem_recv.ty),
                        flow_emit: None,
                    });
                }
                current = elem_recv;
                continue;
            }
        }
        let member = match lookup_member_on(lookup, arena, current, &seg.name, &|_kind| true) {
            Some(m) => m,
            None => return Err(member_miss_cause(lookup, arena, current)),
        };
        if i == last {
            // The final member's yield type (with the receiver's type arguments
            // substituted) records x's type for `const x = a.b.c()` so a later
            // `x.method()` can root on it — forward inference compounds.
            //
            // A terminal call (`obj.method(args)`) carries the call on the ref
            // itself (`kind=Calls`), not on the last segment, so `seg.is_call` is
            // false there. Apply it: a binding `const x = obj.method(args)` must
            // yield the method's RETURN type, not the method-as-value field type.
            let terminal_is_call = seg.is_call
                || matches!(ref_ctx.extracted_ref.kind, crate::types::EdgeKind::Calls);
            let resolved_yield_type =
                yield_through(lookup, arena, &member, terminal_is_call, current.ty, current.id);
            return Ok(SymbolInfo {
                target_symbol_id: member.id,
                confidence: RESOLVED_CONFIDENCE,
                strategy: STRATEGY,
                resolved_yield_type,
                flow_emit: None,
            });
        }
        // Advance to the type this member yields, substituting the receiver's
        // type arguments for the declaring type's generic parameters, then
        // expand it through any type alias before the next member lookup.
        // Pin the new receiver's id using the member's package context so the
        // chain stays anchored to the package the use site established rather
        // than falling back to a first-winner by_qname re-search.
        let yielded = match yield_through(lookup, arena, &member, seg.is_call, current.ty, current.id) {
            Some(y) => y,
            None => {
                // The member itself was found — the hop dies because ITS OWN
                // return/field type was never captured, the same shape as a
                // chain-root miss but discovered one hop in.
                let kind = if seg.is_call { CauseKind::UncapturedReturn } else { CauseKind::UncapturedField };
                return Err(Some(Cause::new(Some(member.id), kind)));
            }
        };
        current = expand_receiver(yielded_receiver(lookup, arena, yielded, member.package_id), lookup, arena, Some(file_ctx));
    }
    Err(None)
}

/// Classify why `lookup_member_on` found nothing on `recv`, using only the
/// receiver state the chain walker already holds — no re-resolution.
///
/// A bound declaration id is decisive: an external declaration with zero
/// materialized members means the externals pipeline never exposed this
/// type's surface (`ExternalUnmaterialized`); any other bound declaration
/// with no matching member genuinely lacks it (`MemberMissing`). With no
/// bound id, the receiver's head may still name a capture-only alias arm
/// (Union / Intersection / Keyof / Other) that member lookup can never
/// expand into a member set (`AliasOpaque`); anything else is indeterminate
/// and left uncaused rather than guessed.
fn member_miss_cause(lookup: &dyn SymbolLookup, arena: &TypeArena, recv: Receiver) -> Option<Cause> {
    if let Some(id) = recv.id {
        let sym = lookup.symbol_by_id(id)?;
        let has_members = !lookup.members_of_id(id).is_empty()
            || !lookup.members_of(&sym.qualified_name).is_empty();
        return Some(if sym.file_path.starts_with("ext:") && !has_members {
            Cause::new(Some(id), CauseKind::ExternalUnmaterialized)
        } else {
            Cause::new(Some(id), CauseKind::MemberMissing)
        });
    }
    let head = head_qname(arena, recv.ty)?;
    match lookup.alias_target(&head) {
        Some(AliasTargetIds::Union(_))
        | Some(AliasTargetIds::Intersection(_))
        | Some(AliasTargetIds::Keyof(_))
        | Some(AliasTargetIds::Other) => Some(Cause::new(
            lookup.by_qualified_name(&head).map(|s| s.id),
            CauseKind::AliasOpaque,
        )),
        _ => None,
    }
}

/// A chain receiver: the type a segment evaluates to, plus the symbol id of the
/// declaration that type names when one is indexed. The id is the identity spine
/// of the walk — member lookup and the supertype climb key on it so two
/// declarations sharing a qname string never collide.
#[derive(Clone, Copy)]
pub(crate) struct Receiver {
    ty: TypeId,
    id: Option<i64>,
}

impl Receiver {
    /// A receiver whose declaration is known by id.
    fn new(ty: TypeId, id: i64) -> Self {
        Self { ty, id: Some(id) }
    }

    /// A receiver typed but not yet bound to a declaration id; the id is
    /// recovered from the type's head when `expand_receiver` runs.
    fn untyped(ty: TypeId) -> Self {
        Self { ty, id: None }
    }
}

/// Expand a receiver's type through any type alias, then resolve the declaration
/// id for the (possibly rewritten) head. When alias expansion changes the head,
/// the receiver names a different declaration — re-derive the id from the new
/// head via `by_qualified_name`. When the head is unchanged (no alias was
/// applied), the caller's established id is more specific than a first-winner
/// re-search: keep it and only fall back to `by_qualified_name` when no id was
/// carried. This prevents a same-qname collision in another package from
/// overwriting an id that was pinned at the use site or by prior-hop package
/// context.
fn expand_receiver(
    recv: Receiver,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file_ctx: Option<&FileContext>,
) -> Receiver {
    let pre_head = head_qname(arena, recv.ty);
    // Expand through the carried id when present, so a bare-name alias collision
    // (two sibling `type Logger = …`) expands the declaration the use site pinned,
    // not the name map's last writer.
    let ty = alias::expand_with_id(recv.ty, recv.id, lookup, arena);
    let post_head = head_qname(arena, ty);
    let id = if pre_head != post_head {
        // Alias rewrote the head — re-derive from the new head, fall back to
        // the carried id when the new head has no indexed declaration.
        head_symbol_id(arena, lookup, ty, file_ctx).or(recv.id)
    } else {
        // Head unchanged — the caller's id is the authoritative identity; the
        // by_qname first-winner is only a fallback for id-less (external/ambient)
        // receivers.
        recv.id.or_else(|| head_symbol_id(arena, lookup, ty, file_ctx))
    };
    // Re-root a bare simple-name head onto its indexed (package-prefixed) declaration
    // so the member walk's qname-keyed lookups resolve — the symmetry the
    // static-access root already has.
    reroot_bare_head(arena, lookup, ty, id, file_ctx)
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
fn reroot_bare_head(
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    ty: TypeId,
    id: Option<i64>,
    file_ctx: Option<&FileContext>,
) -> Receiver {
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
fn rebind_head(arena: &TypeArena, ty: TypeId, qname: &str) -> TypeId {
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

/// The symbol id of the declaration a type's nominal head names: the exact-qname
/// declaration, or — for a simple-name head (an annotation root like `Page`) that
/// matches several type declarations across packages — the one the use site's
/// imports scope to. `None` for a head with no indexed declaration
/// (external/ambient/string-parsed) or when imports don't disambiguate; the walk
/// then falls back to qname-string member lookup.
fn head_symbol_id(
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    ty: TypeId,
    file_ctx: Option<&FileContext>,
) -> Option<i64> {
    let head = head_qname(arena, ty)?;
    if let Some(s) = lookup.by_qualified_name(&head) {
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

/// The declaration id a type's simple-name head resolves to under the use site's
/// import scope, but ONLY when the name is AMBIGUOUS (several type declarations
/// share it). A unique name returns `None` — the name-keyed alias map resolves
/// those correctly, so the scoped pick is skipped. Disambiguates which of two
/// sibling aliases (`type Logger = …` in two files) the use site imported, so
/// `expand_with_id` keys on its id rather than the name map's last writer.
///
/// The import that brings the name into the file names its source module. A
/// path-aliased internal module (`@/utils/logger`) is rewritten through
/// `resolve_path_alias` to a file the candidate is matched against — decisive
/// where same-package siblings tie on path proximity. Falls back to scored
/// ranking when the name is not imported (ambient / same-file) or no file matches.
fn import_scoped_decl_id(
    ref_ctx: &RefContext,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    ty: TypeId,
) -> Option<i64> {
    let head = head_qname(arena, ty)?;
    let simple = head.rsplit('.').next().unwrap_or(&head);
    let types = lookup.types_by_name(simple);
    let candidates: Vec<&Symbol> = types.iter().collect();
    if candidates.len() < 2 {
        return None;
    }
    if let Some(module) = file_ctx
        .imports
        .iter()
        .find(|i| i.imported_name == simple || i.alias.as_deref() == Some(simple))
        .and_then(|i| i.module_path.as_deref())
    {
        let resolved = lookup
            .resolve_path_alias(ref_ctx.file_package_id, module)
            .unwrap_or_else(|| module.to_string());
        let mut matched: Option<&Symbol> = None;
        for cand in &candidates {
            if file_matches_module(&cand.file_path, &resolved) {
                if matched.is_some() {
                    matched = None; // two files match — defer to scored ranking
                    break;
                }
                matched = Some(cand);
            }
        }
        if let Some(s) = matched {
            return Some(s.id);
        }
    }
    pick_ranked_candidate(file_ctx, ref_ctx.file_package_id, lookup, &candidates).map(|s| s.id)
}

/// A candidate file satisfies an import module specifier when the file path —
/// extension and a trailing `/index` dropped — matches the specifier's path tail
/// (`./` / `../` / leading `/` trimmed). `apps/web/utils/logger.ts` matches
/// `./utils/logger` (a `@/utils/logger` import post path-alias rewrite).
fn file_matches_module(file_path: &str, module: &str) -> bool {
    let fp = file_path.replace('\\', "/");
    let fp = fp.rsplit_once('.').map(|(b, _)| b).unwrap_or(&fp);
    let fp = fp.strip_suffix("/index").unwrap_or(fp);
    let tail = module
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches("../")
        .trim_start_matches('/')
        .to_string();
    !tail.is_empty() && (fp == tail || fp.ends_with(&format!("/{tail}")))
}

/// Build a `Receiver` for the type a member yielded, pinning the declaration id
/// from the member's package context when possible. A same-qname type in the
/// member's package is preferred over `by_qualified_name`'s first-winner, so the
/// chain stays anchored to the package the use site established. Falls back to
/// `by_qualified_name` when no package id is recorded on the member or when no
/// same-package declaration exists (external/ambient heads).
fn yielded_receiver(
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
/// Falls back to `by_qualified_name` (first-winner) for external/ambient heads
/// that have no indexed declaration in the preferred package.
fn head_symbol_id_preferring_package(
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    ty: TypeId,
    preferred_pkg: Option<i64>,
) -> Option<i64> {
    let head = head_qname(arena, ty)?;
    let simple = head.rsplit('.').next().unwrap_or(&head);
    if let Some(pkg) = preferred_pkg {
        for s in lookup.by_name(simple) {
            if s.qualified_name == head && s.package_id == Some(pkg) {
                return Some(s.id);
            }
        }
    }
    lookup.by_qualified_name(&head).map(|s| s.id)
}

/// Find `member` on a receiver, preferring the identity path: when the receiver
/// is bound to a declaration id, climb its supertypes by id via
/// `lookup_member_by_id`; otherwise fall back to the qname-string climb. The
/// id path is what keeps two same-qname receiver types apart.
fn lookup_member_on(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    recv: Receiver,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
) -> Option<Symbol> {
    let result = lookup_member_on_bounded(lookup, arena, recv, member, accept, MAX_MAPPED_DEPTH);
    let recv_head = head_qname(arena, recv.ty).unwrap_or_default();
    crate::tracef!(
        "  MEMBER '{}' on receiver={} (recv.id={:?}) -> {}",
        member,
        recv_head,
        recv.id,
        result.as_ref().map(|s| s.qualified_name.as_str()).unwrap_or("NOT FOUND"),
    );
    result
}

/// The type of `field` accessed on a receiver of type `recv_ty` (declaration id
/// `recv_id` when known) — `R.field`'s type for a destructured binding
/// `const { field } = <expr-of-type-R>`. Expands R through any alias, finds the
/// member, and yields its field type with R's type arguments substituted. `None`
/// when R carries no such field or its type can't be walked. Reuses the full
/// member walk (supertype climb, intersection / union / mapped / constructor
/// fallbacks), so a field on a destructured binding resolves exactly as a member
/// step in a chain would.
pub(crate) fn field_type_on(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    recv_ty: TypeId,
    recv_id: Option<i64>,
    field: &str,
) -> Option<TypeId> {
    let recv = expand_receiver(Receiver { ty: recv_ty, id: recv_id }, lookup, arena, None);
    let member = lookup_member_on(lookup, arena, recv, field, &|_kind| true)?;
    let yielded = yield_through(lookup, arena, &member, false, recv.ty, recv.id)?;
    // Normalize the field's type — peel a `NoInfer<T>` intrinsic wrapper (and any
    // transparent alias) so the recorded binding type is the concrete `T`.
    Some(alias::expand(yielded, lookup, arena))
}

/// Upper bound on mapped-type source hops — a mapped type whose source is itself
/// a mapped type chains here. Bounds the rare nesting and guards a cyclic alias.
const MAX_MAPPED_DEPTH: usize = 6;

/// `lookup_member_on` with a mapped-type recursion budget. After the direct and
/// supertype-climb lookups miss, a receiver that is a MAPPED type
/// (`{ [K in keyof T]: V }` — `Mapped<Src>`) resolves the
/// member on its SOURCE object: the mapped type's keys ARE the source's keys, so
/// `mapped.member` IS `source.member`. The source param is bound to the
/// receiver's applied type argument, then the member walk recurses on it.
fn lookup_member_on_bounded(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    recv: Receiver,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
    depth: usize,
) -> Option<Symbol> {
    if let Some(id) = recv.id {
        if let Some(m) = lookup_member_by_id(lookup, id, member, accept) {
            return Some(m);
        }
    }
    // Composite receiver: a union / intersection TYPE (not a named alias) has no
    // nominal head, so the head-keyed walks below cannot see it. Resolve the
    // member across the arms directly. An INTERSECTION carries every branch's
    // members (TS `&`), so the member resolves on any one arm. A UNION admits only
    // members present on every arm (TS union access), so each arm must carry it;
    // the first arm's resolution is returned once all agree. The depth budget
    // bounds a cyclic arm that re-expands to the composite.
    if depth > 0 {
        match arena.get(recv.ty) {
            Type::Intersection(arms) => {
                for arm in arms {
                    let arm_recv = expand_receiver(Receiver::untyped(arm), lookup, arena, None);
                    if let Some(m) =
                        lookup_member_on_bounded(lookup, arena, arm_recv, member, accept, depth - 1)
                    {
                        return Some(m);
                    }
                }
                return None;
            }
            Type::Union(arms) => {
                if arms.is_empty() {
                    return None;
                }
                let mut resolved: Option<Symbol> = None;
                for arm in arms {
                    let arm_recv = expand_receiver(Receiver::untyped(arm), lookup, arena, None);
                    match lookup_member_on_bounded(lookup, arena, arm_recv, member, accept, depth - 1)
                    {
                        None => return None,
                        Some(m) => {
                            if resolved.is_none() {
                                resolved = Some(m);
                            }
                        }
                    }
                }
                return resolved;
            }
            _ => {}
        }
    }
    let head = head_qname(arena, recv.ty)?;
    if let Some(m) = lookup_member(lookup, &head, member, accept) {
        return Some(m);
    }
    if depth == 0 {
        return None;
    }
    // Intersection alias `A & B & {…}` carries every member of every branch (TS
    // `&` semantics). After the direct / id / supertype climbs miss, resolve the
    // member on each NAMED branch — anonymous object branches are already
    // flattened into the alias's own members, so only the named heads remain.
    if let Some(m) = lookup_member_on_intersection(lookup, arena, &head, member, accept, depth) {
        return Some(m);
    }
    // Union alias `A | B | …`: TS union member access is valid only for members
    // present on EVERY arm — typically all arms extend a shared base that
    // declares the member. Resolve it on each arm; the shared declaration is the
    // result.
    if let Some(m) = lookup_member_on_union(lookup, arena, &head, member, accept, depth) {
        return Some(m);
    }
    // Primitive-name capitalization: a head like `"string"` / `"number"` /
    // `"boolean"` has no members keyed under the lowercase name, but the
    // nominal wrapper class (`"String"`, `"Number"`, `"Boolean"`) carries the
    // full member index. When the lowercase head has no indexed members, try
    // its capitalized form. This is universally sound — a head with an
    // UPPERCASE first letter is already handled by the ordinary member walk
    // above; only a lowercase-first head that looks like a built-in primitive
    // triggers this. A capitalized probe that finds nothing is a no-op.
    if let Some(m) = lookup_member_on_capitalized_primitive(lookup, &head, member, accept) {
        return Some(m);
    }
    // Static-member receiver: a builtin value `Foo` (`Object`, `Promise`, `Date`,
    // `Array`, `Number`, …) carries its static members on the co-named
    // `${Foo}Constructor` interface, not on the instance `interface Foo`. TS encodes
    // the value/type split as `declare var Foo: FooConstructor`, so `Foo.staticM`
    // resolves on the constructor interface. After the instance side misses, retry on
    // `${head}Constructor`. A head with no indexed `${head}Constructor` (every
    // non-builtin head) finds nothing — a no-op.
    if let Some(m) = lookup_member_on_constructor_interface(lookup, &head, member, accept) {
        return Some(m);
    }
    // Namespace-qualified receiver: a head like `Ns.Type` whose interface is
    // indexed under the bare last segment (`Type`) — common for codegen that
    // re-exports a per-file type through a wrapper namespace. After the dotted
    // head misses, resolve the member on the bare segment.
    if let Some(m) = lookup_member_on_namespaced(lookup, &head, member, accept) {
        return Some(m);
    }
    if let Some(source_ty) = mapped_source_type(lookup, arena, recv.ty, &head) {
        let source_recv = expand_receiver(Receiver::untyped(source_ty), lookup, arena, None);
        // A mapped source that resolves back to the mapped type itself makes no
        // progress — skip rather than spin to the depth bound.
        if head_qname(arena, source_recv.ty).as_deref() != Some(head.as_str()) {
            if let Some(m) =
                lookup_member_on_bounded(lookup, arena, source_recv, member, accept, depth - 1)
            {
                return Some(m);
            }
        }
    }
    // A MAPPED-ALIAS supertype: the receiver `extends` a mapped-type alias
    // (`interface Recv extends Mapped<Src, T>`), which declares no own members, so
    // the flat supertype climb above missed it. Resolve through the supertype's
    // mapped source, carrying the edge args.
    if let Some(m) = lookup_member_on_mapped_supertype(lookup, arena, &head, member, accept, depth) {
        return Some(m);
    }
    // The mapped source is an UNBOUND parameter: it falls back to its declared
    // default (a `typeof <namespace>`) whose keys are the mapped object's members,
    // resolved through the receiver's wildcard re-export closure.
    lookup_member_via_unbound_mapped_source(lookup, arena, recv.ty, &head, member, accept)
}

/// Resolve `member` on a MAPPED-ALIAS supertype of the receiver. A type can
/// `extends` a mapped-type alias (`interface Recv extends Mapped<Src, T>`); the
/// alias declares no members of its own, so the
/// ordinary supertype climb (flat `members_of`) misses it. Build the
/// supertype's applied type from the `extends` edge args and resolve `member`
/// through its mapped source — the same path a mapped RECEIVER takes. `None`
/// when no direct parent is a mapped alias or none carries the member.
fn lookup_member_on_mapped_supertype(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    head: &str,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
    depth: usize,
) -> Option<Symbol> {
    for parent_head in lookup.parent_class_qnames(head) {
        let is_mapped = matches!(
            lookup.alias_target(parent_head),
            Some(AliasTargetIds::Mapped { .. }) | Some(AliasTargetIds::IntersectionMapped { .. })
        );
        if !is_mapped {
            continue;
        }
        // The supertype's applied type, carrying the `extends Parent<Arg>` edge
        // args so its mapped source param binds to the concrete argument.
        let base = arena.class(parent_head);
        // Prefer the interned-id edge args; fall back to interning the string form.
        let id_slice = lookup.parent_class_arg_ids(head, parent_head);
        let arg_ids: Vec<TypeId> = if !id_slice.is_empty() {
            id_slice.to_vec()
        } else {
            lookup
                .parent_class_args(head, parent_head)
                .iter()
                .map(|a| arena.intern_type_str(a))
                .collect()
        };
        let parent_ty = if arg_ids.is_empty() {
            base
        } else {
            arena.intern(Type::Apply { base, args: arg_ids })
        };
        if let Some(source_ty) = mapped_source_type(lookup, arena, parent_ty, parent_head) {
            let source_recv = expand_receiver(Receiver::untyped(source_ty), lookup, arena, None);
            if head_qname(arena, source_recv.ty).as_deref() != Some(parent_head.as_str()) {
                if let Some(m) =
                    lookup_member_on_bounded(lookup, arena, source_recv, member, accept, depth - 1)
                {
                    return Some(m);
                }
            }
        }
        if let Some(m) = lookup_member_via_unbound_mapped_source(
            lookup, arena, parent_ty, parent_head, member, accept,
        ) {
            return Some(m);
        }
    }
    None
}

/// When `head` begins with a lowercase letter, probe the same name with its
/// first letter uppercased. Covers the primitive-to-nominal promotion for
/// languages (TypeScript, Java, Kotlin, Swift, …) where lowercase primitive
/// names (`string`, `int`, `boolean`) have a corresponding boxed/nominal
/// class (`String`, `Int`, `Boolean`) that carries the member index.
///
/// Only fires when the regular lowercase lookup already missed — this is a
/// last-resort fallback before the mapped-source hop, not a pre-pass.
/// Returns `None` when the head is already uppercase or the capitalized probe
/// finds no member.
fn lookup_member_on_capitalized_primitive(
    lookup: &dyn SymbolLookup,
    head: &str,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
) -> Option<Symbol> {
    let first = head.chars().next()?;
    if !first.is_lowercase() {
        return None;
    }
    let capitalized = {
        let mut s = String::with_capacity(head.len());
        s.extend(first.to_uppercase());
        s.push_str(&head[first.len_utf8()..]);
        s
    };
    lookup_member(lookup, &capitalized, member, accept)
}

/// Static-member fallback: a builtin value `Foo` carries its static members on the
/// co-named `${Foo}Constructor` interface (`Object.keys` → `ObjectConstructor.keys`,
/// `Promise.resolve` → `PromiseConstructor.resolve`, `Date.now` → `DateConstructor.now`).
/// TS declares the value as `declare var Foo: FooConstructor`; the instance
/// `interface Foo` holds none of these. Resolve `member` on `${head}Constructor`.
/// `None` when the head already ends in `Constructor` (no double-suffix probe) or no
/// such interface is indexed.
fn lookup_member_on_constructor_interface(
    lookup: &dyn SymbolLookup,
    head: &str,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
) -> Option<Symbol> {
    if head.ends_with("Constructor") {
        return None;
    }
    let ctor = format!("{head}Constructor");
    lookup_member(lookup, &ctor, member, accept)
}

/// Namespace-qualified receiver fallback: a head like `Ns.Type` whose members are
/// indexed under the bare last segment (`Type`) — the type is declared in its own
/// file and surfaced through a wrapper namespace's re-export, so its members are
/// keyed on the bare name. Resolve `member` on the last `.`-segment. `None` for an
/// unqualified head or an empty trailing segment.
fn lookup_member_on_namespaced(
    lookup: &dyn SymbolLookup,
    head: &str,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
) -> Option<Symbol> {
    let (_, last) = head.rsplit_once('.')?;
    if last.is_empty() {
        return None;
    }
    // Suffix-qualify: an external extends-clause / mapped-source arg names a type
    // by its package-LOCAL dotted name (`Ns.Type`, where `Ns` is imported from
    // another package), but the type is indexed package-prefixed
    // (`@scope/pkg.Ns.Type`). Resolve `member` on the type-like symbol whose qname
    // IS `head` or ends with `.head`, climbing its supertypes by id. More specific
    // than the bare-segment fallback below, so it runs first.
    let suffix = format!(".{head}");
    for cand in lookup.types_by_name(last).iter() {
        if cand.qualified_name == head || cand.qualified_name.ends_with(&suffix) {
            if let Some(m) = lookup_member_by_id(lookup, cand.id, member, accept) {
                return Some(m);
            }
        }
    }
    // Bare last segment as a qname (`Ns.Type` → `Type`).
    lookup_member(lookup, last, member, accept)
}

/// Resolve `member` on the named branches of an intersection alias. A branch is
/// the head name of an `&` member (`Mapped` for `Mapped<Q> & {…}`);
/// anonymous object branches contribute no name and are skipped (their members are
/// flattened onto the alias itself). Each branch name is resolved to its
/// declaration(s) by simple name, then the member walk recurses by symbol id so a
/// branch shared across packages stays distinct and the branch's own supertypes
/// climb. Returns the first branch that carries `member`. `None` when `head` is
/// not an intersection alias or no branch carries the member.
fn lookup_member_on_intersection(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
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
    for &branch_id in &branches {
        // A branch is a NOMINAL type reference (`Class("Foo")`); resolving it to
        // its declaration is by name (`types_by_name`), the same path every type
        // reference uses — multi-candidate, so an ambiguous branch tries each.
        let branch = arena.format_type(branch_id);
        if branch.is_empty() || branch == head {
            continue;
        }
        for cand in lookup.types_by_name(&branch).iter() {
            let recv = expand_receiver(
                Receiver::new(arena.class(&cand.qualified_name), cand.id),
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
fn lookup_member_on_union(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
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
    for &branch_id in &branches {
        // A branch is a NOMINAL type reference; resolve it to its declaration by
        // name (`types_by_name`) — the same path every type reference uses.
        let branch = arena.format_type(branch_id);
        if branch.is_empty() || branch == head {
            // A primitive/literal/self arm cannot carry the member; union access
            // requires it on every arm, so the access is invalid.
            return None;
        }
        let mut branch_hit: Option<Symbol> = None;
        for cand in lookup.types_by_name(&branch).iter() {
            let recv = expand_receiver(
                Receiver::new(arena.class(&cand.qualified_name), cand.id),
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

/// The source object type of a mapped alias `{ [K in keyof Src]: … }`, with the
/// mapped param bound to the receiver's applied type argument: `Override<A, B>`
/// (params `[A, B]`, source `A`) with receiver `Override<MutationObserverResult,
/// …>` yields `MutationObserverResult`. A source naming a concrete type rather
/// than a param is interned directly. `None` when `head` is not a mapped alias
/// or the mapped capture recorded no source.
fn mapped_source_type(
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
fn lookup_member_via_unbound_mapped_source(
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

/// Resolve `member` on `type_qname`, climbing its supertypes up to
/// `MAX_SUPERTYPE_DEPTH`. `accept` gates a candidate by kind. The climb is
/// bounded and cheap, so it is recomputed per call rather than memoized.
pub(crate) fn lookup_member(
    lookup: &dyn SymbolLookup,
    type_qname: &str,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
) -> Option<Symbol> {
    let mut cur = type_qname.to_string();
    for _ in 0..MAX_SUPERTYPE_DEPTH {
        for m in lookup.members_of(&cur) {
            if m.name == member && accept(&m.kind) {
                return Some(m.clone());
            }
        }
        match lookup.parent_class_qname(&cur) {
            Some(parent) => cur = parent.to_string(),
            None => break,
        }
    }
    None
}

/// Resolve `member` on the declaration `type_id`, climbing its supertypes up to
/// `MAX_SUPERTYPE_DEPTH` by SYMBOL ID. Both the member index (`members_of_id`)
/// and the supertype climb (`parent_class_id`) key on identity, so a receiver
/// whose qname is shared by an unrelated type in another package binds members
/// only from THIS declaration and climbs only ITS recorded base — never a
/// first-wins qname collision.
pub(crate) fn lookup_member_by_id(
    lookup: &dyn SymbolLookup,
    type_id: i64,
    member: &str,
    accept: &dyn Fn(&str) -> bool,
) -> Option<Symbol> {
    // Breadth-first over the supertype DAG: a type can extend/implement several
    // supertypes, and the member may be declared on any of them, so every direct
    // parent is followed (not a single linear chain). A visited set prevents
    // re-walking a diamond, and the depth bound caps the climb.
    let mut visited: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut frontier = vec![type_id];
    for _ in 0..MAX_SUPERTYPE_DEPTH {
        let mut next: Vec<i64> = Vec::new();
        for cur in frontier {
            if !visited.insert(cur) {
                continue;
            }
            for m in lookup.members_of_id(cur) {
                if m.name == member && accept(&m.kind) {
                    return Some(m.clone());
                }
            }
            next.extend(lookup.parent_class_ids(cur));
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    None
}

/// The positional index a `tuple_index:N` ComputedAccess segment selects, or
/// `None` for any other segment. Emitted by the array-destructure extractor.
fn tuple_index_of(seg: &crate::types::ChainSegment) -> Option<usize> {
    seg.node_kind.strip_prefix("tuple_index:")?.parse().ok()
}

/// The type of element `idx` of a tuple receiver — a direct `Type::Tuple`, or an
/// alias to one (`Signal<T> = [Accessor<T>, Setter<T>]`). The alias's own generic
/// params are substituted with the receiver's applied arguments so the element
/// carries the bound type (`Signal<boolean>` → element 0 = `Accessor<boolean>`).
/// `None` when the receiver is not a tuple or has no element at `idx`.
fn tuple_element_type(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    recv_ty: TypeId,
    idx: usize,
) -> Option<TypeId> {
    if let Type::Tuple(elems) = arena.get(recv_ty) {
        return elems.get(idx).copied();
    }
    // An array-destructure of an array (`const [a, b] = someArray`) emits the same
    // tuple_index segments; array applications are homogeneous, so every position
    // is the element type.
    if let Some(elem) = array_element_type(arena, recv_ty) {
        return Some(elem);
    }
    let head = head_qname(arena, recv_ty)?;
    let AliasTargetIds::Tuple(elem_ids) = lookup.alias_target(&head)? else {
        return None;
    };
    let elem_ty = *elem_ids.get(idx)?;
    let params = lookup.generic_params(&head).unwrap_or_default();
    let args = apply_args(arena, recv_ty);
    if params.is_empty() || args.is_empty() {
        return Some(elem_ty);
    }
    let map: rustc_hash::FxHashMap<String, TypeId> = params.into_iter().zip(args).collect();
    Some(arena.rebind_class_params(elem_ty, &map))
}

/// The element type of a homogeneous single-arg sequence application — `E` for
/// `Apply(Array,[E])` / `Apply(ReadonlyArray,[E])` / `Apply(Vec,[E])`. `Array`
/// is the canonical head `intern_type_str` mints for every `T[]` suffix and
/// Rust's `[T; N]` / `[T]` array/slice syntax; `Vec` is Rust's growable-vector
/// head, decomposed the same way any `Foo<Bar>` generic application is — so
/// this is language-agnostic. Looks through the nullable/async/iterator
/// wrappers the same way `head_qname` does. `None` for any other receiver —
/// the head is checked against the known homogeneous-sequence heads rather
/// than projecting the first argument of any application, since a keyed
/// (non-sequence) container's subscript does not yield its first type
/// argument. The caller falls through to named-member lookup so a
/// string-keyed `obj['key']` index still resolves as a member.
fn array_element_type(arena: &TypeArena, recv_ty: TypeId) -> Option<TypeId> {
    match arena.get(recv_ty) {
        Type::Apply { base, args } => {
            let head = head_qname(arena, base)?;
            if matches!(head.as_str(), "Array" | "ReadonlyArray" | "Vec") {
                args.first().copied()
            } else {
                None
            }
        }
        Type::Optional(inner) | Type::AsyncWrapper(inner) | Type::Iterator(inner) => {
            array_element_type(arena, inner)
        }
        _ => None,
    }
}

/// The qualified-name head of a type: `Repository` for `Repository<User>`.
/// Looks through generic application and the nullable/async/iterator wrappers so
/// member access on `Promise<User>` / `User?` resolves on `User`. `None` for a
/// structural type with no nominal head (function, tuple, union, …).
pub(crate) fn head_qname(arena: &TypeArena, id: TypeId) -> Option<String> {
    match arena.get(id) {
        Type::Class(q) => Some(q),
        Type::Apply { base, .. } => head_qname(arena, base),
        Type::Optional(inner) | Type::AsyncWrapper(inner) | Type::Iterator(inner) => {
            head_qname(arena, inner)
        }
        _ => None,
    }
}

/// The applied type arguments of a type: `[User]` for `Repository<User>`, empty
/// for a non-generic type. Looks through wrappers the same way `head_qname` does
/// so the args travel with the head they belong to.
pub(crate) fn apply_args(arena: &TypeArena, id: TypeId) -> Vec<TypeId> {
    match arena.get(id) {
        Type::Apply { args, .. } => args,
        Type::Optional(inner) | Type::AsyncWrapper(inner) | Type::Iterator(inner) => {
            apply_args(arena, inner)
        }
        _ => Vec::new(),
    }
}

/// The type `member` yields when accessed through `receiver`, with the
/// receiver's applied type arguments substituted for the declaring type's
/// generic parameters: `Repository<User>` receiver + `find(): T` → `User`.
///
/// Fluent-chain rebind: when the substituted yield type's head is `"this"` or
/// `"Self"`, the member is a fluent step that returns the receiver itself.
/// Substitute the receiver type directly so the next member lookup stays on
/// the receiver's type rather than dead-ending on a nominal `Class("this")`
/// that has no members.
///
/// Callable-property unwrap: when `is_call=true` and the member's declared
/// yield type is a function type (`Type::Function { return_ }`), the call
/// result is the function's own return type — a property whose declared type
/// is a call signature (`fn: () => Mock<T>`) yields `Mock<T>` when called.
fn yield_through(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    member: &Symbol,
    is_call: bool,
    receiver: TypeId,
    recv_id: Option<i64>,
) -> Option<TypeId> {
    let result = yield_through_impl(lookup, arena, member, is_call, receiver, recv_id);
    crate::tracef!(
        "  YIELD '{}' is_call={}: return_type_id={} field_type_id={} -> {}",
        member.qualified_name,
        is_call,
        lookup.return_type_id(&member.qualified_name).map(|id| format!("{id:?}")).as_deref().unwrap_or("None"),
        lookup.field_type_id(&member.qualified_name).map(|id| format!("{id:?}")).as_deref().unwrap_or("None"),
        result.map(|id| format!("{id:?}")).as_deref().unwrap_or("None"),
    );
    result
}

/// Inner body of `yield_through`. See `yield_through` for the contract.
fn yield_through_impl(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    member: &Symbol,
    is_call: bool,
    receiver: TypeId,
    recv_id: Option<i64>,
) -> Option<TypeId> {
    let raw = member_yield_type(lookup, arena, member, is_call)?;
    // Callable-property unwrap: if the raw yield is a function type and the
    // member is being called, peel off one call layer to get the return type.
    // This covers `fn: () => Mock<T>` properties: the field type interns as
    // `Type::Function { return_: Mock<T> }`; calling the property yields
    // `Mock<T>`, not the function-type descriptor itself.
    let raw = if is_call {
        match arena.get(raw) {
            crate::type_checker::core::types::Type::Function { return_, .. } => return_,
            _ => raw,
        }
    } else {
        raw
    };
    // Bind a generic supertype's parameters from the `extends Base<Arg>` edge
    // BEFORE the receiver substitution: a member found on `Base` in
    // `Child extends Base<User>` yields `T`, which the edge args bind to `User`.
    // Those args live on the edge, not the receiver, so `substitute_through`
    // (receiver-args only) cannot see them.
    let raw = substitute_supertype_args(lookup, arena, member, raw, receiver);
    let substituted = substitute_through(lookup, arena, raw, receiver, recv_id);
    // Fluent-chain rebind: `this`/`Self` head means "return the receiver".
    if is_self_head(arena, substituted) {
        return Some(receiver);
    }
    // Covariant mapped-fluent rebind: a chaining getter `g(): Src` declared on
    // `Src`, reached via `interface Recv extends Mapped<Src>` whose mapped value
    // maps such members to `Recv`, must continue the chain on `Recv` — not on the
    // `Src` its signature names. `Recv` carries the members the next step looks up;
    // `Src` (the mapped source) does not.
    if let Some(rebound) = mapped_fluent_rebind(lookup, arena, member, substituted, receiver) {
        return Some(rebound);
    }
    Some(substituted)
}

/// Rebind a member's yield to the RECEIVER when the member is a fluent chaining
/// getter (returns its own declaring type) reached through one of the receiver's
/// MAPPED supertypes. Gated on both conditions so a plain supertype method that
/// returns a fixed base type is unaffected: the yield must equal the member's
/// declaring type, and that type must be the mapped source of a receiver
/// supertype. `None` when either gate fails.
fn mapped_fluent_rebind(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    member: &Symbol,
    yielded: TypeId,
    receiver: TypeId,
) -> Option<TypeId> {
    let decl = member.qualified_name.rsplit_once('.')?.0;
    let yield_head = head_qname(arena, yielded)?;
    if !qnames_same_type(decl, &yield_head) {
        return None;
    }
    let recv_head = head_qname(arena, receiver)?;
    if qnames_same_type(decl, &recv_head) {
        return None;
    }
    if receiver_mapped_supertype_has_source(lookup, arena, &recv_head, decl) {
        return Some(receiver);
    }
    None
}

/// Whether two type names denote the same type, tolerating a package-prefix
/// difference: a package-local name `Ns.Type` and its indexed qname
/// `@scope/pkg.Ns.Type` match. Exact, or one is the dotted suffix of the other.
fn qnames_same_type(a: &str, b: &str) -> bool {
    a == b || a.ends_with(&format!(".{b}")) || b.ends_with(&format!(".{a}"))
}

/// Whether `receiver_head` has a MAPPED supertype whose bound mapped source is
/// `target` — i.e. the receiver reaches `target` through a mapped `extends`
/// (`interface Recv extends Mapped<Src>`, mapped source `Src`). Builds each
/// mapped parent's applied type from its edge args so the source binds to the
/// concrete argument.
fn receiver_mapped_supertype_has_source(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    receiver_head: &str,
    target: &str,
) -> bool {
    for parent_head in lookup.parent_class_qnames(receiver_head) {
        let is_mapped = matches!(
            lookup.alias_target(parent_head),
            Some(AliasTargetIds::Mapped { .. }) | Some(AliasTargetIds::IntersectionMapped { .. })
        );
        if !is_mapped {
            continue;
        }
        let base = arena.class(parent_head);
        // Prefer the interned-id edge args; fall back to interning the string form.
        let id_slice = lookup.parent_class_arg_ids(receiver_head, parent_head);
        let arg_ids: Vec<TypeId> = if !id_slice.is_empty() {
            id_slice.to_vec()
        } else {
            lookup
                .parent_class_args(receiver_head, parent_head)
                .iter()
                .map(|a| arena.intern_type_str(a))
                .collect()
        };
        let parent_ty = if arg_ids.is_empty() {
            base
        } else {
            arena.intern(Type::Apply { base, args: arg_ids })
        };
        if let Some(src) = mapped_source_type(lookup, arena, parent_ty, parent_head) {
            if let Some(src_head) = head_qname(arena, src) {
                if qnames_same_type(target, &src_head) {
                    return true;
                }
            }
        }
    }
    false
}

/// The type `member` yields when used in a chain, as a canonical TypeId: its
/// return type for a call, else its declared field/property type.
///
/// Reads the id-keyed `*_id_of` slot FIRST so a member whose qname is shared
/// across packages yields THIS declaration's type, not the qname first-winner;
/// falls back to the qname string accessors for an id-less member (external /
/// ambient, with no id-slot type recorded). `None` when the index has no type
/// for it.
///
/// Callable-property fallback: when `is_call=true` and no explicit return type
/// is recorded, the member may be a property whose declared type is a function
/// signature (e.g. `fn: () => Mock<T>`). In that case the field type is
/// returned unchanged; `yield_through` peels the `Type::Function` wrapper to
/// extract the call result.
pub(crate) fn member_yield_type(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    member: &Symbol,
    is_call: bool,
) -> Option<TypeId> {
    let qname = member.qualified_name.as_str();
    if is_call {
        if let Some(id) = lookup
            .return_type_id_of(member.id)
            .or_else(|| lookup.return_type_id(qname))
        {
            return Some(id);
        }
        if let Some(s) = lookup.return_type_str(qname) {
            return Some(arena.intern_type_str(&s));
        }
        // A synthesized object-literal return type (`{fn}$Ret`): the function's
        // body returned an object literal, materialized post-extract as a type
        // whose members are the object's properties. A call yields that type.
        let synth_ret = format!("{qname}$Ret");
        if lookup.by_qualified_name(&synth_ret).is_some() {
            return Some(arena.class(&synth_ret));
        }
        // No explicit return type — the member's declared (field) type may be
        // callable. An inline function type (`fn: () => Mock`) is returned as-is
        // for `yield_through` to peel its `Type::Function` wrapper. A field type
        // that NAMES a function/method symbol (a property typed `typeof someFn`)
        // interns as a nominal head, not a `Type::Function`, so resolve it to
        // the named function's own return type here.
        let ft = lookup
            .field_type_id_of(member.id)
            .or_else(|| lookup.field_type_id(qname))
            .or_else(|| lookup.field_type_str(qname).map(|s| arena.intern_type_str(&s)))?;
        if let Some(head) = head_qname(arena, ft) {
            if let Some(rt) = callable_named_return(lookup, arena, &head) {
                return Some(rt);
            }
        }
        return Some(ft);
    }
    if let Some(id) = lookup
        .field_type_id_of(member.id)
        .or_else(|| lookup.field_type_id(qname))
    {
        return Some(id);
    }
    if let Some(s) = lookup.field_type_str(qname) {
        return Some(arena.intern_type_str(&s));
    }
    // A getter (`get user(): T`) is indexed as a `method` but accessed as a
    // PROPERTY — `obj.user` (no call) yields its declared RETURN type, not a
    // field type. A member with a return type but no field type is a getter; a
    // bare method reference whose chain continues roots on the same return, so
    // this fallback only adds yields, never replaces a field/return a call needs.
    if let Some(id) = lookup
        .return_type_id_of(member.id)
        .or_else(|| lookup.return_type_id(qname))
    {
        return Some(id);
    }
    lookup.return_type_str(qname).map(|s| arena.intern_type_str(&s))
}

/// `true` when the type's nominal head is the `this` or `Self` keyword —
/// the fluent-return convention across TypeScript (`this`), Rust (`Self`),
/// Swift (`Self`), and Kotlin (`this`). When a member's yield type has this
/// head, the chain advances to the RECEIVER's type instead.
fn is_self_head(arena: &TypeArena, id: TypeId) -> bool {
    match head_qname(arena, id) {
        Some(h) => h == "this" || h == "Self",
        None => false,
    }
}

/// Substitute the receiver's applied type arguments for the declaring type's
/// generic parameters throughout `yielded`. `Repository<User>` receiver with
/// params `[T]` rebinds `Class("T")` → `User`; a non-generic receiver, or a
/// declaring type with no parameters, leaves `yielded` untouched.
fn substitute_through(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    yielded: TypeId,
    receiver: TypeId,
    recv_id: Option<i64>,
) -> TypeId {
    let args = apply_args(arena, receiver);
    if args.is_empty() {
        return yielded;
    }
    // Prefer the receiver's declaration id to read its generic parameters
    // directly; fall back to rendering the receiver's nominal head only when the
    // receiver has no bound declaration (external / ambient / string-parsed).
    let params: Vec<String> = match recv_id.and_then(|id| lookup.generic_params_of(id)) {
        Some(p) => p,
        None => match head_qname(arena, receiver) {
            Some(head) => lookup.generic_params(&head).unwrap_or_default(),
            None => return yielded,
        },
    };
    if params.is_empty() {
        return yielded;
    }
    let map: FxHashMap<String, TypeId> =
        params.into_iter().zip(args.iter().copied()).collect();
    arena.rebind_class_params(yielded, &map)
}

/// Bind a generic SUPERTYPE's parameters from the `extends`/`implements` edge
/// when `member` was found on that supertype, not on the receiver itself:
/// `class Child extends Base<User>` + `Base.m: T` yields `T`, which the edge
/// args `[User]` bind to `User`. The args ride on the edge (`inherits_args`),
/// not on the receiver type, so `substitute_through` — which reads only the
/// receiver's own applied args — cannot supply them.
///
/// No-op when the receiver has no nominal head, the member is declared on the
/// receiver type itself (then `substitute_through` already handles it), the edge
/// records no args, or the supertype has no generic parameters. Single-hop: the
/// member's declaring type must be a DIRECT supertype of the receiver head.
fn substitute_supertype_args(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    member: &Symbol,
    yielded: TypeId,
    receiver: TypeId,
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
    // Prefer the interned-id form of the edge args; fall back to interning the
    // stored arg strings for id-less stores (incremental reload / test fixtures).
    let id_slice = lookup.parent_class_arg_ids(&recv_head, decl_head);
    let arg_ids: Vec<TypeId> = if !id_slice.is_empty() {
        id_slice.to_vec()
    } else {
        lookup
            .parent_class_args(&recv_head, decl_head)
            .iter()
            .map(|a| arena.intern_type_str(a))
            .collect()
    };
    if arg_ids.is_empty() {
        return yielded;
    }
    let params = lookup.generic_params(decl_head).unwrap_or_default();
    if params.is_empty() {
        return yielded;
    }
    let map: FxHashMap<String, TypeId> =
        params.into_iter().zip(arg_ids.iter().copied()).collect();
    arena.rebind_class_params(yielded, &map)
}

/// Type the chain's root segment. Structural cases only for now:
///   - a local variable's forward-inferred type (`local_type` cache),
///   - a declared annotation captured on the segment (head + split type args),
///   - `this`/`self`/`base` → the enclosing type,
///   - a call root (`makeRepo()`) → the callee's return type,
///   - an imported / typed value whose declaration carries a type,
///   - a bare type name used as a static-access / construction root.
/// Each case interns its type expression into the arena once, here at the root.
///
/// Returns a `Receiver`: the root type plus, where a SPECIFIC declaration is in
/// hand (the bare type name resolved through `types_by_name`, the enclosing type
/// for `this`/`self`), that declaration's symbol id. Cases that produce a type
/// EXPRESSION (a local's inferred type, a declared annotation, a value's field
/// type, a callee return) leave the id unbound; `expand_receiver` recovers it
/// from the type's head. The id is what keeps a same-named receiver type in one
/// package distinct from another's during the member walk.
fn resolve_root(
    ref_ctx: &RefContext,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    seg: &crate::types::ChainSegment,
) -> Result<Receiver, Option<Cause>> {
    let result = resolve_root_impl(ref_ctx, file_ctx, lookup, arena, seg);
    let result_str = result.as_ref().ok().map_or_else(|| "UNTYPABLE".to_string(), |r| {
        head_qname(arena, r.ty)
            .map(|h| format!("typed {h}"))
            .unwrap_or_else(|| "typed (structural)".to_string())
    });
    crate::tracef!(
        "  ROOT '{}': local_type_id={} local_type={} declared_type={} is_call={} -> {}",
        seg.name,
        lookup.local_type_id(&seg.name).map(|id| format!("{id:?}")).as_deref().unwrap_or("None"),
        lookup.local_type(&seg.name).as_deref().unwrap_or("None"),
        seg.declared_type.as_deref().unwrap_or("None"),
        seg.is_call,
        result_str,
    );
    result
}

/// Inner body of `resolve_root`. See `resolve_root` for the contract.
fn resolve_root_impl(
    ref_ctx: &RefContext,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    seg: &crate::types::ChainSegment,
) -> Result<Receiver, Option<Cause>> {
    // Prefer the TypeId cache: `local_type_id` returns the TypeId that was
    // stored directly by `record_local_type_id`, preserving the exact type
    // variant (Primitive, Optional, Generic) without a format/intern round-trip.
    // Fall back to the String cache and intern once at this boundary when no
    // TypeId binding exists (e.g. bindings recorded via the String path or by
    // synthetic test doubles that only implement `local_type`).
    // A binding typed `ReturnType<typeof fn>` (or any return-type-extraction alias
    // of that shape) roots on `fn`'s return type, resolved in this file's import
    // scope so the right overload is chosen. Applies to both the TypeId-cached and
    // String-cached local bindings.
    if let Some(id) = lookup.local_type_id(&seg.name) {
        if let Some(r) = resolve_return_type_extraction(id, lookup, arena, file_ctx) {
            return Ok(Receiver::untyped(r));
        }
        return Ok(Receiver::untyped(id));
    }
    if let Some(ty) = lookup.local_type(&seg.name) {
        let id = arena.intern_type_str(&ty);
        if let Some(r) = resolve_return_type_extraction(id, lookup, arena, file_ctx) {
            return Ok(Receiver::untyped(r));
        }
        return Ok(Receiver::untyped(id));
    }
    if let Some(ty) = &seg.declared_type {
        return Ok(Receiver::untyped(with_segment_args(
            arena,
            arena.intern_type_str(ty),
            &seg.type_args,
        )));
    }
    if matches!(seg.kind, SegmentKind::SelfRef) {
        // `this`/`self` roots on the enclosing type — the one declaration whose
        // members the chain walks. Bind its id so an inherited-member climb keys
        // on identity, not the enclosing type's qname string.
        if let Some(enc_qname) =
            lookup.enclosing_type_qname(&ref_ctx.source_symbol.qualified_name)
        {
            let ty = arena.class(enc_qname);
            let id = lookup.by_qualified_name(enc_qname).map(|s| s.id);
            return Ok(Receiver { ty, id });
        }
        // `enclosing_type_qname` walks `parent_index`, which is empty for a
        // language whose methods are extracted as AST siblings of their
        // container rather than nested children (Rust impl blocks). Fall back
        // to the scope chain (innermost first), then the source symbol's own
        // `scope_path`, accepting the first type-kind symbol either names.
        let enc_sym = ref_ctx
            .scope_chain
            .iter()
            .find_map(|q| lookup.by_qualified_name(q).filter(|s| is_type_kind(&s.kind)))
            .or_else(|| {
                let sp = ref_ctx.source_symbol.scope_path.as_deref()?;
                lookup.by_qualified_name(sp).filter(|s| is_type_kind(&s.kind))
            })
            .ok_or(None)?;
        let ty = arena.class(&enc_sym.qualified_name);
        return Ok(Receiver { ty, id: Some(enc_sym.id) });
    }
    // An externally-imported root binds to the imported module's declaration of
    // the name, never a same-named symbol from a different external package: a `z`
    // imported from `zod` roots on zod's `z`, not a DOM `CSSRotate.z`; an `expect`
    // from `vitest` on vitest's, not playwright's. The import names both the
    // identifier AND its source module, so it disambiguates a collision the bare
    // by-name fallbacks below cannot. Fires only when the imported module actually
    // declares the name (an `ext:` file under that module); otherwise those
    // fallbacks run unchanged.
    if let Some(recv) = import_scoped_external_root(file_ctx, lookup, arena, seg) {
        return Ok(recv);
    }

    // Nothing typed the root outright. The remaining strategies may still find
    // the SYMBOL the root names — just discover its own type was never
    // captured. Keep the most specific such near-miss; it beats a blank
    // unbound_root when every strategy below also comes up empty.
    let mut cause: Option<Cause> = None;

    if seg.is_call {
        match resolve_callee_return_and_id(lookup, arena, file_ctx, &seg.name, None) {
            Ok((ty, _id)) => return Ok(Receiver::untyped(ty)),
            Err(c) => cause = cause.or(c),
        }
        // The callee is not a callable declaration (function/method) but may be
        // a VALUE whose declared type is a callable interface — `const v: I`
        // where `I` carries a call signature. Calling it yields the call
        // signature's return, not the interface type itself, so this must precede
        // the value-root fallthrough below.
        if let Some(ty) = call_value_root_type(
            lookup,
            arena,
            &seg.name,
            &ref_ctx.source_symbol.qualified_name,
            file_ctx,
            ref_ctx.file_package_id,
        ) {
            return Ok(Receiver::untyped(ty));
        }
    }
    // Import-of-value / typed-value root: a value (a `declare const`, an
    // imported binding, a typed field) whose declaration carries a type roots
    // the chain on that type — `builder.create()` roots on `builder`'s declared
    // type even though `builder` is not itself a type name.
    match value_root_type(
        lookup,
        arena,
        &seg.name,
        &ref_ctx.source_symbol.qualified_name,
        file_ctx,
        ref_ctx.file_package_id,
    ) {
        Ok(ty) => {
            // A value whose declared type is `ReturnType<typeof f>` — an inferred
            // `const x = f(...)` binding imported from the file that declares it —
            // roots on f's return type (f resolved globally by name), deriving the
            // cross-file type the per-file flow seed could not carry.
            if let Some(r) = resolve_return_type_extraction(ty, lookup, arena, file_ctx) {
                return Ok(Receiver::untyped(r));
            }
            return Ok(Receiver::untyped(ty));
        }
        Err(c) => cause = cause.or(c),
    }
    // Bare type name used as a static-access / construction root. When the same
    // name is declared in several sibling workspace packages, prefer the
    // declaration in the package the use site imports the name from; otherwise
    // fall back to the first same-named type. The resolved `Symbol` IS the
    // receiver's declaration, so bind its id directly rather than round-tripping
    // its qname back through `by_qualified_name`.
    let candidates = lookup.types_by_name(&seg.name);
    let cand_refs: Vec<&Symbol> = candidates.iter().collect();
    // Prefer the import-scoped declaration — the package/module the use site
    // imports `name` from — over a first-winner same-name pick (a `Page` from the
    // package the file imports, not a same-named `Page` in another). Ranked +
    // ascending-id deterministic; when no candidate clearly wins, the first
    // same-named type is the fallback.
    let Some(s) = pick_ranked_candidate(file_ctx, ref_ctx.file_package_id, lookup, &cand_refs)
        .or_else(|| candidates.first())
    else {
        // A prior forward-inference pass may have already diagnosed why THIS
        // EXACT binding carries no seeded type — that names the true upstream
        // cause (an initializer's uncaptured return/field), which outranks the
        // generic "this binding itself is untyped" signal collected above.
        return Err(lookup.root_cause_hint(&seg.name).or(cause));
    };
    let ty = with_segment_args(arena, arena.class(&s.qualified_name), &seg.type_args);
    Ok(Receiver::new(ty, s.id))
}

/// Reduce an import specifier to its package root: `next/server` → `next`,
/// `@scope/pkg/sub` → `@scope/pkg`, `zod` → `zod`. A scoped package keeps its
/// first two slash segments; an unscoped one keeps the first.
fn package_root(specifier: &str) -> &str {
    if specifier.starts_with('@') {
        // `@scope/pkg…` — the root is everything up to the second `/`.
        let mut slashes = 0;
        for (i, c) in specifier.char_indices() {
            if c == '/' {
                slashes += 1;
                if slashes == 2 {
                    return &specifier[..i];
                }
            }
        }
        specifier
    } else {
        match specifier.find('/') {
            Some(i) => &specifier[..i],
            None => specifier,
        }
    }
}

/// `true` when an external symbol's `ext:<lang>:<modpath>` file path sits under
/// the package `root`: `ext:ts:vitest/globals.d.ts` is under `vitest`,
/// `ext:ts:@tanstack/react-query/build/x.d.ts` under `@tanstack/react-query`. The
/// module path keeps the literal specifier (slashes included), so this match is
/// uniform across scoped and unscoped packages where a qname-prefix test is not.
fn ext_file_under_module(file_path: &str, root: &str) -> bool {
    let Some(rest) = file_path.strip_prefix("ext:") else {
        return false;
    };
    // Drop the `<lang>:` segment (`ts:`, `js:`, …) to reach the module path.
    let Some(colon) = rest.find(':') else {
        return false;
    };
    let modpath = &rest[colon + 1..];
    modpath == root || modpath.strip_prefix(root).is_some_and(|r| r.starts_with('/'))
}

/// The package root the chain-root `name` is imported from, when the import is an
/// EXTERNAL bare specifier (not a relative `./…` path). Relative imports return
/// None — their declarations are not `ext:` files, so the scoped filter would be
/// empty anyway; skipping them avoids the work.
fn external_import_root<'a>(file_ctx: &'a FileContext, name: &str) -> Option<&'a str> {
    for import in &file_ctx.imports {
        if import.imported_name == name || import.alias.as_deref() == Some(name) {
            let spec = import.module_path.as_deref()?;
            if spec.starts_with('.') {
                return None;
            }
            return Some(package_root(spec));
        }
    }
    None
}

/// Root a chain on the imported module's declaration of `name`. See the call site
/// in `resolve_root_impl` for why an import attribution gates the pick. Returns the
/// typed receiver, or None when `name` is not externally imported or the module's
/// declaration of it can't be typed (the caller keeps its generic fallbacks).
fn import_scoped_external_root(
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    seg: &crate::types::ChainSegment,
) -> Option<Receiver> {
    let root = external_import_root(file_ctx, &seg.name)?;
    let by_name = lookup.by_name(&seg.name);
    let scoped: Vec<&Symbol> = by_name
        .iter()
        .filter(|s| ext_file_under_module(&s.file_path, root))
        .collect();
    if scoped.is_empty() {
        return None;
    }
    if seg.is_call {
        // A callable declaration in the module yields its return type.
        for s in scoped.iter().filter(|s| is_callable(&s.kind)) {
            if let Some(id) = lookup
                .return_type_id_of(s.id)
                .or_else(|| lookup.return_type_id(&s.qualified_name))
            {
                return Some(Receiver::untyped(id));
            }
        }
        // A value whose declared type is a callable interface (`const expect:
        // ExpectStatic`) yields that interface's call-signature return.
        for s in scoped.iter().filter(|s| is_value_kind(&s.kind)) {
            let Some(vty) = field_type_of(lookup, arena, s.id, &s.qualified_name) else {
                continue;
            };
            let recv = expand_receiver(Receiver::untyped(vty), lookup, arena, None);
            if let Some(call) =
                lookup_member_on(lookup, arena, recv, CALL_SIGNATURE_MEMBER, &|_| true)
            {
                if let Some(r) = yield_through(lookup, arena, &call, true, recv.ty, recv.id) {
                    return Some(Receiver::untyped(r));
                }
            }
        }
        return None;
    }
    // Non-call root: the imported value's own declared type, else the namespace /
    // type declaration itself (its members are walked for `z.string`).
    for s in &scoped {
        if is_value_kind(&s.kind) {
            if let Some(id) = field_type_of(lookup, arena, s.id, &s.qualified_name) {
                return Some(Receiver::untyped(id));
            }
        }
    }
    let s = scoped.first()?;
    Some(Receiver::new(arena.class(&s.qualified_name), s.id))
}

/// The package that DECLARES `name` as an ambient global — read off the declaring
/// file of the ambient-scope registration (`__npm_globals__.<name>` sourced from
/// `vitest/globals.d.ts` → `vitest`). `None` when `name` is not a registered
/// global. Lets an unimported chain root prefer that package's own typed
/// declaration over a same-named export of an unrelated package.
fn ambient_global_package(lookup: &dyn SymbolLookup, name: &str) -> Option<String> {
    lookup.ambient_symbols(name).iter().find_map(|s| {
        crate::ecosystem::externals::ts_package_from_virtual_path(&s.file_path)
            .map(|p| p.to_string())
    })
}

/// Type a chain root that is a *value* by the declared type on its declaration.
/// Runs after the flow-cache and annotation roots, so a local's inferred type
/// still wins; this catches values whose type lives on the declaration rather
/// than at the use site.
///
/// Binds the root to its SPECIFIC in-scope declaration first: a local/field is
/// declared in an enclosing scope, so `{scope}.{name}` — derived from the use
/// site's owning symbol, innermost out — names the exact declaration whose type
/// to read, not a same-named value elsewhere in the project. `source_qname` is
/// the symbol owning the reference, so its own qname is the innermost scope (the
/// declaration is its child). Only when no in-scope binding exists do we fall
/// back to the first same-named value anywhere (an imported / ambient / top-level
/// value with no enclosing-scope qname relative to the use site).
fn value_root_type(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    name: &str,
    source_qname: &str,
    file_ctx: &FileContext,
    file_package_id: Option<i64>,
) -> Result<TypeId, Option<Cause>> {
    // A value root's type comes from a binding this file is entitled to root on
    // ("owned"): a scope-qualified match, the same file, an imported name, or an
    // external/ambient declaration (`ext:` — where imports and globals resolve). A
    // *different* internal file's same-name value is only a fallback when this file
    // has NO owned binding of the name (a sole declaration, or a language with
    // implicit cross-file scope). Borrowing it OVER an owned-but-untyped binding is
    // the first-winner leak that mis-typed `logger`/`z`/`response` to an unrelated
    // same-name value (a foreign `logger` typed `Array`).
    // The module `name` is imported from, if any. An import from an INTERNAL module
    // — a relative path or a tsconfig path alias (`@/lib/toast`) — makes a same-name
    // EXTERNAL (`ext:`) symbol foreign, never the imported declaration; without this
    // an untyped internal binding (an uncaptured `Object.assign(...)` value) is
    // overridden by a borrowed external same-name's type. An import from a bare
    // package keeps the blanket ext: ownership that cross-package re-exports rely on
    // (`useQuery` imported from `@tanstack/react-query`, declared in a sibling
    // `@tanstack/*` package), as does a name that is not imported at all (ambient
    // global / bare external type reference).
    let imported_module = file_ctx
        .imports
        .iter()
        .find(|i| i.imported_name == name || i.alias.as_deref() == Some(name))
        .and_then(|i| i.module_path.as_deref());
    let name_imported = imported_module.is_some();
    let import_is_internal = imported_module.is_some_and(|m| {
        m.starts_with('.') || lookup.resolve_path_alias(file_package_id, m).is_some()
    });
    let src_file = file_ctx.file_path.as_str();
    let is_owned = |s: &Symbol| {
        if &*s.file_path == src_file {
            return true;
        }
        if s.file_path.starts_with("ext:") {
            return !import_is_internal;
        }
        name_imported
    };

    // Ambient-global scope: a name used WITHOUT an import that is registered as a
    // global resolves to the package that DECLARES the global, not a same-named
    // export of an unrelated package — `expect` is globalized by vitest, so it
    // roots on vitest's `ExpectStatic`, not playwright's imported `expect`. Prefer
    // that package's own typed value declaration. The ambient registry IS a scope
    // (like import scope), so this never reorders an unscoped multi-candidate root.
    if !name_imported {
        if let Some(pkg) = ambient_global_package(lookup, name) {
            let prefix = format!("{pkg}.");
            for cand in lookup.by_name(name) {
                if !is_value_kind(&cand.kind) || !cand.qualified_name.starts_with(&prefix) {
                    continue;
                }
                if let Some(id) = field_type_of(lookup, arena, cand.id, &cand.qualified_name) {
                    if !is_primitive_head(arena, id) {
                        return Ok(id);
                    }
                }
            }
        }
    }

    // True once an owned same-name binding is seen (even untyped) — then a foreign
    // value's type is never borrowed over it.
    let mut owned_seen = false;
    // The first owned-but-untyped binding's own cause — the diagnosable
    // near-miss returned when nothing else types the root.
    let mut owned_cause: Option<Cause> = None;
    // Non-primitive type from a foreign-internal same-name value, used only when no
    // owned binding exists.
    let mut foreign_fallback: Option<TypeId> = None;
    // A value typed by a bare primitive (an HTTP header's `expect: string`) is a
    // poor chain root; taken only when nothing richer carries the name.
    let mut primitive_fallback: Option<TypeId> = None;

    let mut scope = source_qname;
    loop {
        let qn = if scope.is_empty() {
            name.to_string()
        } else {
            format!("{scope}.{name}")
        };
        if let Some(s) = lookup.by_qualified_name(&qn) {
            if is_value_kind(&s.kind) {
                // A scope-qualified hit carries the source's own scope, so it is
                // owned regardless of file; a bare-name hit (scope exhausted) is a
                // global pick subject to the owned/foreign split.
                let owned = !scope.is_empty() || is_owned(s);
                let ft = field_type_of(lookup, arena, s.id, &qn);
                if owned {
                    if let Some(id) = ft {
                        return Ok(id);
                    }
                    owned_seen = true;
                    owned_cause.get_or_insert_with(|| root_binding_cause(s));
                } else if let Some(id) = ft {
                    if is_primitive_head(arena, id) {
                        primitive_fallback.get_or_insert(id);
                    } else {
                        foreign_fallback.get_or_insert(id);
                    }
                }
            }
        }
        if scope.is_empty() {
            break;
        }
        scope = match scope.rfind('.') {
            Some(i) => &scope[..i],
            None => "",
        };
    }
    for cand in lookup.by_name(name) {
        if !is_value_kind(&cand.kind) {
            continue;
        }
        let owned = is_owned(cand);
        let ft = field_type_of(lookup, arena, cand.id, &cand.qualified_name);
        match ft {
            Some(id) if owned => {
                if is_primitive_head(arena, id) {
                    primitive_fallback.get_or_insert(id);
                    owned_seen = true;
                } else {
                    return Ok(id);
                }
            }
            Some(id) => {
                if is_primitive_head(arena, id) {
                    primitive_fallback.get_or_insert(id);
                } else {
                    foreign_fallback.get_or_insert(id);
                }
            }
            None if owned => {
                owned_seen = true;
                owned_cause.get_or_insert_with(|| root_binding_cause(cand));
            }
            None => {}
        }
    }
    // An owned same-name binding exists ⇒ it roots the chain; never borrow a
    // foreign-internal value's type over it.
    if owned_seen {
        return primitive_fallback.ok_or(owned_cause);
    }
    foreign_fallback.or(primitive_fallback).ok_or(None)
}

/// The cause for an owned root binding found by name but carrying no captured
/// type: a field/property is `UncapturedField`, anything else value-kind
/// (variable, constant, parameter) is `UntypedBinding`.
fn root_binding_cause(sym: &Symbol) -> Cause {
    let kind = match sym.kind.as_str() {
        "field" | "property" => CauseKind::UncapturedField,
        _ => CauseKind::UntypedBinding,
    };
    Cause::new(Some(sym.id), kind)
}

/// `true` when the type's nominal head is a language primitive — a value typed by
/// one of these is a poor chain root, so a same-name primitive-typed property
/// must not shadow a richer same-name value.
fn is_primitive_head(arena: &TypeArena, id: TypeId) -> bool {
    matches!(
        head_qname(arena, id).as_deref(),
        Some(
            "string"
                | "number"
                | "boolean"
                | "bigint"
                | "symbol"
                | "void"
                | "never"
                | "undefined"
                | "null"
                | "unknown"
                | "any"
                | "object"
        )
    )
}

/// The interned field/property type of `qname`: the `field_type_id` if present,
/// else the string accessor interned at this boundary.
///
/// When the resulting type's nominal head names a VALUE rather than a type — the
/// shape the extractor lowers `const x: typeof import('m')['k']` to, where the
/// field type is the bare exported name `k` and `k` is a value the module
/// exports — the value's *own* declared type is the real receiver. Follow that
/// indirection so the chain roots on the referenced export's type, not on the
/// (non-existent) type named by the export's identifier.
fn field_type_of(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    sym_id: i64,
    qname: &str,
) -> Option<TypeId> {
    let id = raw_field_type_of(lookup, arena, sym_id, qname)?;
    Some(deref_value_typed(lookup, arena, id, qname))
}

/// The interned field/property type of the value with id `sym_id` / qname
/// `qname`, without value-indirection following: the id-keyed `field_type_id_of`
/// first so a value whose qname is shared across packages reads ITS own field
/// type, then the qname accessors as the id-less fallback.
fn raw_field_type_of(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    sym_id: i64,
    qname: &str,
) -> Option<TypeId> {
    if let Some(id) = lookup
        .field_type_id_of(sym_id)
        .or_else(|| lookup.field_type_id(qname))
    {
        return Some(id);
    }
    lookup.field_type_str(qname).map(|s| arena.intern_type_str(&s))
}

/// Upper bound on value-indirection hops when a field type names a value whose
/// own declared type is the real receiver. Bounds the rare re-export-shim chain
/// and prevents a self-referential field type from looping.
const MAX_VALUE_TYPE_DEREF: usize = 4;

/// Follow a field type whose nominal head names a VALUE through to the type that
/// declares the members. Two indirections collapse here:
///
///   - A head whose own declaration is a member-less value re-export shim — a
///     `Ns.I` variable with no members, re-exporting an interface of the same
///     simple name declared under a different qname — re-roots onto that
///     same-simple-name type declaration, where the members are keyed.
///   - A head that names no type but names a value whose own field type differs
///     (`const x: typeof import('m')['k']` lowered to the bare export name)
///     follows that value's declared type.
///
/// Stops at a type head that already declares members, a value with no field
/// type, a fixpoint (the value's field type names itself), or
/// `MAX_VALUE_TYPE_DEREF` hops. `origin_qname` is the symbol the original field
/// type was read from; it is excluded from the value lookup so a self-typed shim
/// doesn't loop on itself.
fn deref_value_typed(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    id: TypeId,
    origin_qname: &str,
) -> TypeId {
    let mut current = id;
    let mut seen_qname = origin_qname.to_string();
    for _ in 0..MAX_VALUE_TYPE_DEREF {
        let Some(head) = head_qname(arena, current) else {
            return current;
        };
        // A primitive head (`string` / `number`) has no value-indirection — a
        // value coincidentally named `string` (an exported `string` helper) must
        // not hijack the deref and re-root the chain onto an unrelated type.
        if is_primitive_head(arena, current) {
            return current;
        }
        // The maps are simple-name keyed; a qualified head (`Ns.I`) probes them by
        // its trailing segment.
        let simple = head.rsplit('.').next().unwrap_or(&head);
        // A nominal head whose declaration is a member-bearing type is already
        // the receiver. When a same-simple-name type exists but the head's own
        // declaration is a member-less value re-export shim, advance the receiver
        // onto the type declaration that actually holds the members — its qname
        // differs from the shim's, so the member walk would otherwise dead-end on
        // the member-less value.
        if let Some(type_decl) = receiver_type_for_head(lookup, &head, simple) {
            if type_decl.qualified_name == head {
                return current;
            }
            return arena.class(&type_decl.qualified_name);
        }
        // The head names no type; if it names a value (other than the symbol we
        // just came from) whose own field type differs, that value's type is the
        // real receiver.
        let Some(value) = lookup
            .by_name(simple)
            .into_iter()
            .find(|s| is_value_kind(&s.kind) && s.qualified_name != seen_qname)
        else {
            return current;
        };
        let Some(next) = raw_field_type_of(lookup, arena, value.id, &value.qualified_name) else {
            return current;
        };
        if next == current {
            return current;
        }
        current = next;
        seen_qname = value.qualified_name.clone();
    }
    current
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
fn receiver_type_for_head(
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
    // A re-export shell (`vitest.ExpectStatic` re-exporting `@vitest/expect`'s)
    // resolves to the RE-EXPORTED declaration, not an arbitrary same-simple-name
    // type — the re-export names the authoritative source, so `vitest`'s `expect`
    // reaches `@vitest/expect.ExpectStatic`, not a foreign `@types/chai` one.
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

/// `true` when `kind` names a value whose declared type can root a chain.
fn is_value_kind(kind: &str) -> bool {
    matches!(
        kind,
        "variable" | "constant" | "const" | "field" | "property" | "parameter"
    )
}

/// Type a call root whose callee is a VALUE of a callable-interface type, by the
/// return of that interface's call signature: `const v: I` where `I` carries
/// `(x): R` types `v(x)` as `R`. The
/// extractor synthesises that call signature as a member named `call` on the
/// interface; this types the value to its declared interface, then yields the
/// `call` member's return with the receiver's type arguments substituted — the
/// same member-walk and substitution every chain hop uses, so any callable
/// interface's value types its call result generically.
fn call_value_root_type(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    name: &str,
    source_qname: &str,
    file_ctx: &FileContext,
    file_package_id: Option<i64>,
) -> Option<TypeId> {
    let value_ty =
        value_root_type(lookup, arena, name, source_qname, file_ctx, file_package_id).ok()?;
    let recv = expand_receiver(Receiver::untyped(value_ty), lookup, arena, None);
    let call = lookup_member_on(lookup, arena, recv, CALL_SIGNATURE_MEMBER, &|_kind| true)?;
    yield_through(lookup, arena, &call, true, recv.ty, recv.id)
}

/// The name the extractor synthesises for an interface's call signature
/// (`interface F { (x): R }`), surfaced as a member so a value of that interface
/// type yields `R` when called.
const CALL_SIGNATURE_MEMBER: &str = "call";

/// Type a call root by the callee's return type: `makeRepo()` where
/// `makeRepo(): Repository<User>` types the chain head as `Repository<User>`.
///
/// When several callable declarations share `name` across sibling packages,
/// prefer the one in the package the use site imports `name` from, so the chain
/// heads on THAT declaration's return type. Without an import attribution, or
/// when no candidate sits in the imported package, the first callable wins. This
/// is the same import-scoped declaration pick the bare-type-name root uses; no
/// per-library knowledge enters here.
pub(crate) fn callee_return_type(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file_ctx: &FileContext,
    name: &str,
) -> Option<TypeId> {
    resolve_callee_return_and_id(lookup, arena, file_ctx, name, None)
        .ok()
        .map(|(ret, _)| ret)
}

/// `callee_return_type` that, among same-named callables, prefers one declared
/// INSIDE `enclosing` — a nested `function inner(){…}` whose qname is
/// `{enclosing}.{name}`. A factory's `return inner()` means its OWN nested
/// builder, not a same-named declaration in another file/scope; the bare-name
/// path picks an arbitrary namesake (whichever `by_name` yields first).
pub(crate) fn callee_return_type_in_scope(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file_ctx: &FileContext,
    name: &str,
    enclosing: &str,
) -> Option<TypeId> {
    resolve_callee_return_and_id(lookup, arena, file_ctx, name, Some(enclosing))
        .ok()
        .map(|(ret, _)| ret)
}

/// The return type the callable `name` resolves to in this file's import scope,
/// paired with the SYMBOL ID of the declaration it was read from. The id lets a
/// caller read that declaration's generic params (to bind a call's type
/// arguments). See `callee_return_type` for the resolution order — this is its
/// id-carrying form.
fn resolve_callee_return_and_id(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file_ctx: &FileContext,
    name: &str,
    enclosing_scope: Option<&str>,
) -> Result<(TypeId, i64), Option<Cause>> {
    let candidates = lookup.by_name(name);
    // The most specific callable candidate found so far whose own return type
    // was never captured — the diagnosable near-miss if every strategy below
    // falls through without a typed return.
    let mut untyped_callee: Option<i64> = None;

    // Scope preference: a callee declared INSIDE `enclosing` — a nested
    // `function inner(){…}` whose qname is `{enclosing}.{name}` — is the one a
    // factory's `return inner()` means, resolved before the unscoped name
    // fallback picks an arbitrary namesake. Its `$Ret` (object-literal return)
    // is authoritative, same as the general path below.
    if let Some(scoped_qname) = enclosing_scope.map(|e| format!("{e}.{name}")) {
        if let Some(cand) = candidates
            .iter()
            .find(|s| is_callable(&s.kind) && s.qualified_name == scoped_qname)
        {
            let ret_qname = format!("{}$Ret", cand.qualified_name);
            if lookup.by_qualified_name(&ret_qname).is_some() {
                return Ok((arena.class(&ret_qname), cand.id));
            }
            if let Some(id) = lookup
                .return_type_id_of(cand.id)
                .or_else(|| lookup.return_type_id(&cand.qualified_name))
            {
                return Ok((id, cand.id));
            }
            if let Some(s) = lookup.return_type_str(&cand.qualified_name) {
                return Ok((arena.intern_type_str(&s), cand.id));
            }
            // Scoped callee exists but carries no recorded return — fall through.
            untyped_callee = Some(cand.id);
        }
    }
    // An object-literal return synthesized as `{qname}$Ret` (the flow-return-object
    // pass) IS the function's structural return — authoritative over any stored
    // return inferred from a param annotation (`Record`) or a body expression
    // (`Promise`). Prefer it before reading the stored slot.
    for cand in candidates.iter().filter(|s| is_callable(&s.kind)) {
        let ret_qname = format!("{}$Ret", cand.qualified_name);
        if lookup.by_qualified_name(&ret_qname).is_some() {
            return Ok((arena.class(&ret_qname), cand.id));
        }
    }
    // Import-scoped overload set: when `name` is imported from a specific
    // package, the chain heads on THAT package's declaration. An OVERLOADED
    // function records its return on ONE specific signature (often the
    // implementation, not the first overload), so scan the scoped callables for
    // the one carrying a per-id return rather than blindly taking the first —
    // the root-typing parity the call-ref path gets by binding the arg-matched
    // overload. Prefer the id-keyed return over the qname slot, which a
    // same-named declaration in another package may have won.
    if let Some(pkg) = import_scoped_package_id(file_ctx, lookup, name) {
        let scoped: Vec<_> = candidates
            .iter()
            .filter(|s| is_callable(&s.kind) && s.package_id == Some(pkg))
            .collect();
        for s in &scoped {
            if let Some(id) = lookup.return_type_id_of(s.id) {
                return Ok((id, s.id));
            }
        }
        if let Some(callee) = scoped.first() {
            if let Some(id) = lookup.return_type_id(&callee.qualified_name) {
                return Ok((id, callee.id));
            }
            if let Some(n) = lookup.return_type_str(&callee.qualified_name) {
                return Ok((arena.intern_type_str(&n), callee.id));
            }
            untyped_callee.get_or_insert(callee.id);
        }
    }
    // No import attribution (or the scoped set yielded no return): the first
    // free-function declaration of this name. Methods require an explicit receiver
    // and must not root a bare unscoped call — they are excluded here so an
    // unrelated method named the same as an ambient callable-interface const does
    // not shadow the const's call-signature path.
    let Some(callee) = candidates.iter().find(|s| s.kind == "function") else {
        return Err(untyped_callee.map(|id| Cause::new(Some(id), CauseKind::UncapturedReturn)));
    };
    if let Some(id) = lookup.return_type_id_of(callee.id) {
        return Ok((id, callee.id));
    }
    if let Some(id) = lookup.return_type_id(&callee.qualified_name) {
        return Ok((id, callee.id));
    }
    if let Some(s) = lookup.return_type_str(&callee.qualified_name) {
        return Ok((arena.intern_type_str(&s), callee.id));
    }
    Err(Some(Cause::new(Some(callee.id), CauseKind::UncapturedReturn)))
}

/// The return type of a call `name<args>(…)`, with the call's explicit type
/// arguments bound to the callee's generic parameters and substituted into the
/// declared return. A parameter the call leaves unbound takes its default,
/// resolved against parameters already bound (`TData = TQueryFnData`), so a single
/// `useQuery<Movie>` arg flows through to a `TData`-typed return. Falls back to the
/// unsubstituted return when the callee carries no generic params or the call
/// supplies no type arguments.
pub(crate) fn call_return_with_type_args(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file_ctx: &FileContext,
    name: &str,
    call_args: &[TypeId],
) -> Option<TypeId> {
    let (ret, callee_id) = resolve_callee_return_and_id(lookup, arena, file_ctx, name, None).ok()?;
    // The return propagates to every same-qname overload id, but the params are
    // stored only on the declaration that parsed them — which may be a SIBLING of
    // the id whose return was read. Use the callee's own params when present, else
    // borrow an overload sibling's (overloads share their type parameters).
    let params_id = if lookup.generic_params_of(callee_id).is_some() {
        callee_id
    } else {
        lookup
            .by_name(name)
            .iter()
            .find(|s| is_callable(&s.kind) && lookup.generic_params_of(s.id).is_some())
            .map(|s| s.id)
            .unwrap_or(callee_id)
    };
    let Some(params) = lookup.generic_params_of(params_id) else {
        return Some(ret);
    };
    if params.is_empty() || call_args.is_empty() {
        return Some(ret);
    }
    let defaults = lookup.generic_param_defaults_of(params_id).unwrap_or_default();
    let mut subst: FxHashMap<String, TypeId> = FxHashMap::default();
    for (i, param) in params.iter().enumerate() {
        // Positional arg, else the param's default — which may name an earlier
        // param (`TData = TQueryFnData`), resolved against the bindings so far.
        let bound = call_args.get(i).copied().or_else(|| {
            defaults.get(i).and_then(|d| d.as_ref()).map(|d| {
                subst
                    .get(d)
                    .copied()
                    .unwrap_or_else(|| arena.intern_type_str(d))
            })
        });
        if let Some(ty) = bound {
            subst.insert(param.clone(), ty);
        }
    }
    Some(arena.rebind_class_params(ret, &subst))
}

/// Resolve a return-type-extraction application — `ReturnType<typeof fn>`, and
/// any user alias of the same shape — to `fn`'s return type.
///
/// The shape is structural, not name-based (see `is_return_type_extraction`).
/// Applied to `typeof fn`, the checked type is `fn`'s value, so the result is
/// `fn`'s return type, resolved in the use site's import scope via
/// `callee_return_type` so the correct overload / package is selected. `None`
/// when `ty_str` is not such an application, or the callee or its return type
/// can't be resolved.
pub(crate) fn resolve_return_type_extraction(
    ty: TypeId,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file_ctx: &FileContext,
) -> Option<TypeId> {
    let head = head_qname(arena, ty)?;
    // Flat-interned `ReturnType<typeof f>` — a single Class head rather than an
    // `Apply{ReturnType,[…]}` (the inferred `const x = f()` field type, whose
    // `<…>` payload the type-ref capture does not decompose). Parse the wrapped
    // value off the head string and resolve f's return directly.
    if let Some(inner) = head
        .strip_prefix("ReturnType<typeof ")
        .and_then(|s| s.strip_suffix('>'))
    {
        return typeof_value_return_type(inner.trim(), lookup, arena, file_ctx);
    }
    // Either a named alias whose RHS is `ReturnType<…>` (`type Logger =
    // ReturnType<typeof createScopedLogger>`), or the raw `ReturnType` intrinsic
    // applied directly — a variable inferred as `ReturnType<typeof f>` from a
    // `const x = f(...)` initializer, which an importing file resolves on demand.
    let is_rt = head == "ReturnType"
        || lookup
            .alias_target(&head)
            .map(|t| is_return_type_extraction(arena, t))
            .unwrap_or(false);
    if !is_rt {
        return None;
    }
    let arg = apply_args(arena, ty).first().copied()?;
    let value = head_qname(arena, arg)?
        .strip_prefix("typeof ")
        .map(str::trim)?
        .to_string();
    typeof_value_return_type(&value, lookup, arena, file_ctx)
}

/// The return type of the function bound to `value` at the use site. The binding
/// is scoped to the module `value` is IMPORTED from (`import { f } from
/// '@scope/pkg'` → `@scope/pkg.f`), so an externally imported callee resolves to
/// the correct overload rather than a same-named function in another package —
/// `callee_return_type`'s name-only fallback (which
/// only package-scopes WORKSPACE imports) would otherwise pick a first-winner.
/// Falls back to that name-only resolution for a value with no matching import.
fn typeof_value_return_type(
    value: &str,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file_ctx: &FileContext,
) -> Option<TypeId> {
    for import in &file_ctx.imports {
        let names = import.imported_name == value || import.alias.as_deref() == Some(value);
        if !names {
            continue;
        }
        let Some(module) = import.module_path.as_deref() else {
            continue;
        };
        let qname = format!("{module}.{}", import.imported_name);
        if let Some(id) = lookup.return_type_id(&qname) {
            return Some(id);
        }
        if let Some(s) = lookup.return_type_str(&qname) {
            return Some(arena.intern_type_str(&s));
        }
    }
    callee_return_type(lookup, arena, file_ctx, value)
}

/// A `Conditional` of the return-type-extraction shape: `T extends (...) => infer
/// R ? R : …`. Recognised structurally — `extends` ends in `=> infer <V>` and the
/// true branch is `<V>` — so it matches the lib `ReturnType<T>` and any alias
/// written the same way, regardless of name.
fn is_return_type_extraction(arena: &TypeArena, target: &AliasTargetIds) -> bool {
    let AliasTargetIds::Conditional {
        extends,
        true_branch,
        ..
    } = target
    else {
        return false;
    };
    let extends_str = arena.format_type(*extends);
    let Some((_, infer_tail)) = extends_str.rsplit_once("=> infer ") else {
        return false;
    };
    let infer_var = infer_tail
        .trim_start()
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()
        .unwrap_or("");
    if infer_var.is_empty() {
        return false;
    }
    let true_branch_str = arena.format_type(*true_branch);
    infer_var == true_branch_str.trim()
}

/// Attach a segment's in-source type arguments to a freshly-interned bare head
/// that didn't already carry its own. A declared annotation reaches the chain
/// pre-split — `repo: Repository<User>` as head `Repository` + args `[User]` —
/// so the args must be reattached, else generic substitution sees no arguments.
fn with_segment_args(arena: &TypeArena, id: TypeId, type_args: &[String]) -> TypeId {
    if type_args.is_empty() {
        return id;
    }
    match arena.get(id) {
        Type::Class(_) => {
            let args = type_args.iter().map(|a| arena.intern_type_str(a)).collect();
            arena.intern(Type::Apply { base: id, args })
        }
        // Already an application (or another structural type) — the inline args
        // win and the segment's split args are redundant.
        _ => id,
    }
}

/// `true` when `kind` names something a call can root on — a free function or a
/// method whose return type carries the chain forward.
fn is_callable(kind: &str) -> bool {
    matches!(kind, "function" | "method")
}

/// When a called member's field type NAMES a function/method symbol — the shape
/// a property typed `typeof someFn` produces — the call result is that named
/// function's return type. Prefers a qualified-name hit, then the first callable
/// of that simple name. `None` when `head` doesn't resolve to a callable.
pub(crate) fn callable_named_return(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    head: &str,
) -> Option<TypeId> {
    // Keep the callee's id so a function whose qname is shared across packages
    // reads THIS declaration's return, not a first-winner qname re-search.
    let (callee_id, callee_qname) = {
        if let Some(s) = lookup.by_qualified_name(head).filter(|s| is_callable(&s.kind)) {
            (s.id, s.qualified_name.clone())
        } else {
            let set = lookup.by_name(head);
            let s = set.iter().find(|s| is_callable(&s.kind))?;
            (s.id, s.qualified_name.clone())
        }
    };
    if let Some(id) = lookup
        .return_type_id_of(callee_id)
        .or_else(|| lookup.return_type_id(&callee_qname))
    {
        return Some(id);
    }
    lookup
        .return_type_str(&callee_qname)
        .map(|s| arena.intern_type_str(&s))
}

#[cfg(test)]
#[path = "chain_tests.rs"]
mod tests;
