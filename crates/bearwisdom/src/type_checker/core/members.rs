// =============================================================================
// type_checker/core/members.rs — direct + extension member lookup
//
// MembersIndex is built once per indexing run from ParsedFiles. It owns two
// maps keyed by TypeId: `direct` (members declared inside the type's body —
// methods, fields, properties, enum members) and `extensions` (extension
// members declared outside the type, bound by their receiver type). The chain
// walker calls `lookup` at every segment.
//
// Spec: research/architecture/02-engine-internal-architecture.html § Layer 3
//       research/architecture/04-implementation-phases.html § Phase 3
// =============================================================================

use super::types::{Type, TypeArena, TypeId};
use crate::indexer::canonical_form::signature_arity;
use crate::indexer::resolve::engine::{strip_generic_args, SymbolInfo, SymbolLookup};
use crate::type_checker::core::supertype::SupertypeGraph;
use crate::type_checker::core::symbol_types::SymbolTypeMap;
use crate::type_checker::core::symbol_view::SymbolView;
use crate::type_checker::profile::language_profile::{KindCompatibility, LanguageProfile};
use crate::type_checker::subtype::args_assignable;
use crate::types::{EdgeKind, ParsedFile, SymbolKind};
use rustc_hash::{FxHashMap, FxHashSet};
use std::str::FromStr;
use std::sync::Arc;

/// (file_path, parsed_file_symbol_index) → durable DB symbol id. Same shape as
/// `SymbolTypeMap::SymbolIdMap` so a single map can drive both builders.
pub type SymbolIdMap = FxHashMap<(String, usize), i64>;

/// Resolved call-argument types for type-based overload selection, threaded
/// into member lookup on a call segment. `arg_types` are the call's argument
/// types positionally; `symbol_types` reads a candidate's parameter types;
/// `lookup` backs the assignability check's inheritance walk.
#[derive(Clone, Copy)]
pub struct ArgTypes<'a> {
    pub arg_types: &'a [TypeId],
    pub symbol_types: &'a SymbolTypeMap,
    pub lookup: &'a dyn SymbolLookup,
}

/// Per-type member lookup table.
///
/// `direct` carries members declared in the type's body — the canonical
/// "look at the class definition" path. `extensions` carries members declared
/// outside the type that nevertheless become callable as if they were
/// members (C# `static class Ext { static M(this T t) {} }`, Rust
/// `impl Trait for Type { fn m() {} }`). The split lets the lookup walk
/// in two passes (direct wins on tie) without re-scanning the body map.
#[derive(Debug, Default)]
pub struct MembersIndex {
    direct: FxHashMap<TypeId, Vec<SymbolInfo>>,
    extensions: FxHashMap<TypeId, Vec<SymbolInfo>>,
}

impl MembersIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from extraction output.
    ///
    /// Every symbol whose `scope_path` names a parent qname becomes a direct
    /// member of that parent's Class TypeId. Symbols without a `scope_path`
    /// (top-level functions, top-level type declarations) are skipped — the
    /// chain walker resolves them through the namespace / scope path, not via
    /// member lookup.
    pub fn build_from_parsed_files(
        parsed: &[ParsedFile],
        sym_id_map: &SymbolIdMap,
        arena: &TypeArena,
    ) -> Self {
        let mut index = MembersIndex::new();
        index.ingest_files(parsed, sym_id_map, arena);
        index
    }

    /// Fold the members of `parsed` into this index. The full build runs it over
    /// the whole parse set; the resolve loop's incremental path runs it over only
    /// the files an expand iteration appended — byte-identical because `parsed`
    /// is append-only, so the per-parent member Vecs land in the same order.
    pub fn ingest_files(
        &mut self,
        parsed: &[ParsedFile],
        sym_id_map: &SymbolIdMap,
        arena: &TypeArena,
    ) {
        let index = self;
        for pf in parsed {
            // External files (ext: prefix) carry thousands of symbols per
            // dep — for ts-nextjs that's ~1M symbols. Engine chain walks
            // resolve *into* internal types; external symbols stay
            // lookup-only via SymbolIndex. Admitting all of them as direct
            // members is a huge arena.class write storm, so externals are
            // skipped wholesale EXCEPT one narrow set: a default-method body
            // declared inside an external Trait/Interface. A project type that
            // implements such a trait gains a real supertype edge (build_explicit
            // + the impl-container reroute), so the inherited default IS callable
            // on it — but its body symbol lives here in the ext: file. Admitting
            // only members whose owner is an external trait/interface keeps the
            // write set bounded to that tiny reachability-bounded slice; every
            // other external member (a Class/Struct method, the type-defining
            // symbols themselves) stays skipped.
            let is_external = pf.path.starts_with("ext:");
            let ext_trait_scopes = is_external.then(|| trait_interface_qnames(pf));
            let file_path: Arc<str> = Arc::from(pf.path.as_str());
            for (idx, sym) in pf.symbols.iter().enumerate() {
                if let Some(scopes) = &ext_trait_scopes {
                    // External admission gate: the symbol's owner (scope_path)
                    // must name a Trait/Interface declared in THIS ext: file.
                    // Members with no scope (the trait symbol itself, free
                    // functions) and members owned by a non-trait external type
                    // fall out here, before any arena write — including the
                    // csharp/kotlin extension-signature path below, so external
                    // `this T` extensions stay skipped too.
                    match &sym.scope_path {
                        Some(scope) if scopes.contains(scope.as_str()) => {}
                        _ => continue,
                    }
                }
                let Some(&sym_id) = sym_id_map.get(&(pf.path.clone(), idx)) else {
                    continue;
                };
                let info = SymbolInfo {
                    id: sym_id,
                    name: sym.name.clone(),
                    qualified_name: sym.qualified_name.clone(),
                    kind: sym.kind.as_str().to_string(),
                    visibility: sym.visibility.map(|v| v.as_str().to_string()),
                    file_path: file_path.clone(),
                    scope_path: sym.scope_path.clone(),
                    package_id: pf.package_id,
                    signature: sym.signature.clone(),
                };

                // Extension members bind by their receiver type, declared
                // outside the receiver's body. Languages that fold the receiver
                // into the signature as a leading `this <Recv>` parameter expose
                // it the same way regardless of where the function lives — a C#
                // extension sits in a static class (scoped), a Kotlin top-level
                // extension has no scope. Detect the receiver before the scope
                // gate below so both shapes register.
                if extension_receiver_in_signature(&pf.language) {
                    if let Some(ext_type) = sym.signature.as_deref().and_then(this_extension_target)
                    {
                        let ext_ty = arena.class(ext_type);
                        index
                            .extensions
                            .entry(ext_ty)
                            .or_default()
                            .push(info.clone());
                    }
                }

                // Direct membership requires a scope: the symbol is declared
                // inside the parent named by `scope_path`. Top-level symbols
                // (free functions, top-level types, top-level extensions) have
                // none and are resolved through the namespace / scope path.
                let Some(scope) = &sym.scope_path else {
                    continue;
                };
                if scope.is_empty() {
                    continue;
                }
                let parent_ty = arena.class(scope);
                // A member of a generic type carries the impl's type params in
                // its scope (`IndexWriter<D>`). The chain walker reaches this
                // lookup with the receiver typed two ways: a `self` receiver
                // inside the impl keeps the params (`class("IndexWriter<D>")`),
                // while a receiver typed through a return/field/param annotation
                // is normalized to the bare base (generics are stripped at the
                // yield step, `class("IndexWriter")`). Key the member under the
                // bare base too so both receiver shapes find it.
                let bare_scope = strip_generic_args(scope);
                let bare_parent_ty =
                    (bare_scope.as_str() != scope.as_str()).then(|| arena.class(&bare_scope));
                if let Some(bare_ty) = bare_parent_ty {
                    index.direct.entry(bare_ty).or_default().push(info.clone());
                }
                index.direct.entry(parent_ty).or_default().push(info);
            }
        }
    }

    /// Compute the reachability-bounded external type set and admit its
    /// members in one step. Runs after the supertype graph is built (the graph
    /// supplies the internal → external inheritance seed). A no-op when no
    /// external type is reachable, so a single-language build is unaffected.
    pub fn admit_reachable_externals(
        &mut self,
        parsed: &[ParsedFile],
        sym_id_map: &SymbolIdMap,
        arena: &TypeArena,
        supertypes: &SupertypeGraph,
    ) {
        let reachable = reachable_external_types(parsed, arena, supertypes);
        self.ingest_external_reachable(parsed, sym_id_map, arena, &reachable);
    }

    /// Admit the direct members of every external type named in
    /// `reachable_ext_types` (a set of generics-stripped qnames). The first
    /// `ingest_files` pass skips external Class/Struct methods wholesale — a
    /// receiver typed by an external class then carries no members and a chain
    /// stops at its first hop. This second pass relaxes the skip for the
    /// bounded slice of external types an internal symbol actually reaches:
    /// only those land here, so the write set stays |reached types| × methods
    /// rather than the whole dependency tree.
    ///
    /// Append-only and idempotent like `ingest_files`: re-admitting a type
    /// whose members are already present re-pushes the same `SymbolInfo`s in
    /// the same order, so `find_on_chain`'s first-match is unchanged. The
    /// caller (engine build) runs this once after the supertype graph exists,
    /// and the trait/interface admission from the first pass is untouched.
    pub fn ingest_external_reachable(
        &mut self,
        parsed: &[ParsedFile],
        sym_id_map: &SymbolIdMap,
        arena: &TypeArena,
        reachable_ext_types: &FxHashSet<String>,
    ) {
        if reachable_ext_types.is_empty() {
            return;
        }
        // An owner already keyed in `direct` was admitted by the first pass
        // (a trait/interface default) or a prior reachability pass. Re-admitting
        // its members would duplicate entries and shift first-match order, so
        // those owners are excluded WHOLESALE here — decided once up front, not
        // per-symbol, so a second member of a freshly-admitted owner is not
        // mistaken for an already-keyed owner.
        let already_keyed: FxHashSet<&str> = reachable_ext_types
            .iter()
            .filter(|q| {
                arena
                    .class_lookup(q)
                    .is_some_and(|ty| self.direct.contains_key(&ty))
            })
            .map(|q| q.as_str())
            .collect();
        for pf in parsed {
            if !pf.path.starts_with("ext:") {
                continue;
            }
            let file_path: Arc<str> = Arc::from(pf.path.as_str());
            for (idx, sym) in pf.symbols.iter().enumerate() {
                // Direct membership requires a scope naming the owner type;
                // the owner must be a reachable external type. A scope-less
                // symbol (the type-defining symbol itself, a free function) is
                // not a member and is skipped.
                let Some(scope) = &sym.scope_path else {
                    continue;
                };
                if scope.is_empty() {
                    continue;
                }
                let bare_scope = strip_generic_args(scope);
                if !reachable_ext_types.contains(bare_scope.as_str())
                    || already_keyed.contains(bare_scope.as_str())
                {
                    continue;
                }
                let parent_ty = arena.class(scope);
                let bare_parent_ty =
                    (bare_scope.as_str() != scope.as_str()).then(|| arena.class(&bare_scope));
                let Some(&sym_id) = sym_id_map.get(&(pf.path.clone(), idx)) else {
                    continue;
                };
                let info = SymbolInfo {
                    id: sym_id,
                    name: sym.name.clone(),
                    qualified_name: sym.qualified_name.clone(),
                    kind: sym.kind.as_str().to_string(),
                    visibility: sym.visibility.map(|v| v.as_str().to_string()),
                    file_path: file_path.clone(),
                    scope_path: sym.scope_path.clone(),
                    package_id: pf.package_id,
                    signature: sym.signature.clone(),
                };
                if let Some(bare_ty) = bare_parent_ty {
                    self.direct.entry(bare_ty).or_default().push(info.clone());
                }
                self.direct.entry(parent_ty).or_default().push(info);
            }
        }
    }

    /// Number of types with at least one direct member recorded.
    pub fn direct_type_count(&self) -> usize {
        self.direct.len()
    }

    /// Number of types with at least one extension member recorded.
    pub fn extension_type_count(&self) -> usize {
        self.extensions.len()
    }

    /// All direct members for `ty`, in insertion order. Empty slice when no
    /// members are registered.
    pub fn direct_of(&self, ty: TypeId) -> &[SymbolInfo] {
        self.direct.get(&ty).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// All extension members for `ty`. Same contract as `direct_of`.
    pub fn extensions_of(&self, ty: TypeId) -> &[SymbolInfo] {
        self.extensions
            .get(&ty)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Iterate every TypeId with at least one registered direct member.
    /// Used by structural-supertype discovery to walk the candidate space
    /// without re-scanning all parsed files.
    pub fn direct_keys(&self) -> impl Iterator<Item = TypeId> + '_ {
        self.direct.keys().copied()
    }

    /// Register an extension member against `ty`. Used by per-language hooks
    /// (Rust impl-for-trait, C# extension methods) that discover extension
    /// relationships outside the type's own scope_path.
    pub fn add_extension(&mut self, ty: TypeId, info: SymbolInfo) {
        self.extensions.entry(ty).or_default().push(info);
    }

    /// Register a direct member explicitly. Used by tests and per-language
    /// hooks that synthesize members (decorator-driven, Lua metatable, etc.)
    /// outside the standard scope_path path.
    pub fn add_direct(&mut self, ty: TypeId, info: SymbolInfo) {
        self.direct.entry(ty).or_default().push(info);
    }

    /// Find a member named `name` on `ty` matching `kind_filter`.
    ///
    /// The lookup is total over the Type enum — every variant has a defined
    /// behavior. Returns `None` rather than panicking even on the Unknown
    /// engine-bailout type.
    ///
    /// Recursion:
    /// - `Apply { base, .. }`: look on `base`. Generic substitution of the
    ///   result's declared type happens at the chain walker's yield step
    ///   (Phase 4), not here — the member set is structurally the same
    ///   across all applications of a generic class.
    /// - `Union(branches)`: every branch must carry a kind-compatible member;
    ///   returns the first branch's match (chain walker may narrow further).
    /// - `Intersection(branches)`: any branch with a match wins.
    /// - `Optional(inner)` when profile.look_through_optional: peel and
    ///   recurse.
    /// - `AsyncWrapper(inner)` / `Iterator(inner)`: peel and recurse — the
    ///   wrapper itself has no surface members the engine resolves.
    /// - `Class(_)` / `Primitive(_)`: walk supertype graph BFS and search
    ///   direct + extensions on each ancestor.
    /// - `Generic { param }`: resolve members on the param's declared upper
    ///   bound (`T: Animal` → look up on Animal); unbounded `T` has none.
    /// - `Function` / `Tuple` / `Literal` / `Unknown`: no members.
    pub fn lookup(
        &self,
        ty: TypeId,
        name: &str,
        kind_filter: EdgeKind,
        supertypes: &SupertypeGraph,
        arena: &TypeArena,
        profile: &LanguageProfile,
    ) -> Option<SymbolInfo> {
        self.lookup_with_binding(
            ty,
            name,
            kind_filter,
            supertypes,
            arena,
            profile,
            None,
            None,
        )
        .map(|(sym, _, _)| sym)
    }

    /// Like `lookup`, but also returns the ancestor TypeId the member was
    /// resolved on and the generic arguments bound on the edge that reached
    /// it (both empty/irrelevant when the member is direct or the edge is
    /// non-generic). The chain walker uses these to bind an inherited generic
    /// method's parameters — `class UserRepo: Repository<User>` calling
    /// `Repository::find_one(): T` yields `User`, not unbound `T`.
    ///
    /// `arg_count` is the argument count at a call site (`Some(n)`), used to
    /// disambiguate same-name overloads on one type by arity; `None` (property
    /// access, or arity not extracted) preserves first-match behavior.
    pub fn lookup_with_binding(
        &self,
        ty: TypeId,
        name: &str,
        kind_filter: EdgeKind,
        supertypes: &SupertypeGraph,
        arena: &TypeArena,
        profile: &LanguageProfile,
        arg_count: Option<usize>,
        types: Option<ArgTypes>,
    ) -> Option<(SymbolInfo, TypeId, Vec<TypeId>)> {
        match arena.get(ty) {
            Type::Apply { base, .. } => self.lookup_with_binding(
                base,
                name,
                kind_filter,
                supertypes,
                arena,
                profile,
                arg_count,
                types,
            ),
            Type::Union(branches) => {
                // Every branch must carry the member — partial union members
                // are unsafe to resolve since the runtime value could land
                // on a branch missing the member. Returns the first branch's
                // match; the walker does not yet select a branch by a guard.
                let mut first: Option<(SymbolInfo, TypeId, Vec<TypeId>)> = None;
                for b in branches {
                    match self.lookup_with_binding(
                        b,
                        name,
                        kind_filter,
                        supertypes,
                        arena,
                        profile,
                        arg_count,
                        types,
                    ) {
                        Some(s) => {
                            if first.is_none() {
                                first = Some(s);
                            }
                        }
                        None => return None,
                    }
                }
                first
            }
            Type::Intersection(branches) => {
                for b in branches {
                    if let Some(s) = self.lookup_with_binding(
                        b,
                        name,
                        kind_filter,
                        supertypes,
                        arena,
                        profile,
                        arg_count,
                        types,
                    ) {
                        return Some(s);
                    }
                }
                None
            }
            Type::Optional(inner) if profile.look_through_optional => self.lookup_with_binding(
                inner,
                name,
                kind_filter,
                supertypes,
                arena,
                profile,
                arg_count,
                types,
            ),
            Type::AsyncWrapper(inner) => self.lookup_with_binding(
                inner,
                name,
                kind_filter,
                supertypes,
                arena,
                profile,
                arg_count,
                types,
            ),
            Type::Iterator(inner) => self.lookup_with_binding(
                inner,
                name,
                kind_filter,
                supertypes,
                arena,
                profile,
                arg_count,
                types,
            ),
            Type::Class(_) | Type::Primitive(_) => self.find_on_chain(
                ty,
                name,
                kind_filter,
                supertypes,
                arena,
                profile,
                arg_count,
                types,
            ),
            // A bare generic parameter carries members only through its
            // declared upper bound: `T: Animal` resolves `T`'s members on
            // Animal. Recursion terminates because a bound is a Class/Apply
            // in every realistic declaration; an unbounded `T` has no members.
            Type::Generic { param } => match arena.generic_param(param).bound {
                Some(bound) => self.lookup_with_binding(
                    bound,
                    name,
                    kind_filter,
                    supertypes,
                    arena,
                    profile,
                    arg_count,
                    types,
                ),
                None => None,
            },
            Type::Function { .. }
            | Type::Tuple(_)
            | Type::Literal(_)
            | Type::Optional(_)
            | Type::Unknown => None,
        }
    }

    /// Walk the supertype chain starting at `ty` and find a kind-compatible
    /// member named `name`. Direct members win over extension members at the
    /// same supertype level — extensions extend but do not override the body.
    /// When `arg_count` is `Some(n)`, a same-name overload on the matched
    /// ancestor whose signature declares `n` parameters is preferred; with no
    /// arity match (or `None`) the first kind-compatible member wins, which is
    /// the behavior for non-call segments and languages that don't extract call
    /// arguments. When `types` is supplied, a same-arity overload whose
    /// parameter types accept the argument types wins over the bare arity match.
    /// Selection stays within one ancestor so inheritance overrides
    /// are unaffected. Returns the member, the ancestor it was found on, and
    /// the generic arguments bound on the edge that reached that ancestor.
    fn find_on_chain(
        &self,
        ty: TypeId,
        name: &str,
        kind_filter: EdgeKind,
        supertypes: &SupertypeGraph,
        arena: &TypeArena,
        profile: &LanguageProfile,
        arg_count: Option<usize>,
        types: Option<ArgTypes>,
    ) -> Option<(SymbolInfo, TypeId, Vec<TypeId>)> {
        for (ancestor, args) in supertypes.linearize_with_args(ty, arena, profile.ancestor_order) {
            let mut first: Option<&SymbolInfo> = None;
            let mut arity_hit: Option<&SymbolInfo> = None;
            let mut type_hit: Option<&SymbolInfo> = None;
            for s in self
                .direct_of(ancestor)
                .iter()
                .chain(self.extensions_of(ancestor).iter())
            {
                if s.name != name || !kind_matches(profile, kind_filter, &s.kind) {
                    continue;
                }
                if first.is_none() {
                    first = Some(s);
                }
                let arity_match = arg_count.is_some()
                    && s.signature.as_deref().and_then(signature_arity) == arg_count;
                if arity_match && arity_hit.is_none() {
                    arity_hit = Some(s);
                }
                // Among arity-matching overloads, prefer the one whose declared
                // parameter types accept the call's argument types.
                if arity_match && type_hit.is_none() {
                    if let Some(t) = types {
                        if let Some(params) = SymbolView::new(s, t.symbol_types).param_types() {
                            if !params.is_empty()
                                && args_assignable(
                                    params,
                                    t.arg_types,
                                    arena,
                                    t.lookup,
                                    self,
                                    t.symbol_types,
                                    profile.primitive_mapping,
                                )
                            {
                                type_hit = Some(s);
                            }
                        }
                    }
                }
            }
            if let Some(found) = type_hit.or(arity_hit).or(first) {
                return Some((found.clone(), ancestor, args));
            }
        }
        None
    }
}

/// Map a SymbolInfo's stringified kind back to `SymbolKind` and consult the
/// profile's compatibility table. Unrecognized kinds (synthetic strings the
/// extractor coined for non-standard symbols) default to permissive — a
/// stricter answer would let an extractor typo silently hide real symbols.
fn kind_matches(profile: &LanguageProfile, edge: EdgeKind, sym_kind: &str) -> bool {
    let Ok(parsed) = SymbolKind::from_str(sym_kind) else {
        return true;
    };
    KindCompatibility::check(profile.kind_compatible_table, edge, parsed)
}

/// Qualified names of every Trait / Interface symbol declared in `pf`. A
/// member's `scope_path` is matched against this set to admit an external
/// trait/interface default-method body into the direct-member map; nothing
/// else from an external file is admitted. A non-trait external type's methods
/// produce no entry here, so they stay skipped (the write-storm guard).
fn trait_interface_qnames(pf: &ParsedFile) -> FxHashSet<&str> {
    pf.symbols
        .iter()
        .filter(|s| matches!(s.kind, SymbolKind::Trait | SymbolKind::Interface))
        .map(|s| s.qualified_name.as_str())
        .collect()
}

/// The generics-stripped qnames of external types an internal symbol reaches,
/// closed over the type edges between external types so a chain can walk past
/// the first external hop.
///
/// Seed (depth 1 from project code):
///   - the parent of every supertype edge whose child is internal and whose
///     parent is an external type (a project class extending an external base);
///   - every external type named as the declared / return / parameter type of
///     an internal symbol (a field/local/return typed by an external class).
///
/// Closure: an admitted external type's own members yield further external
/// types (`Kysely.selectFrom(): SelectQueryBuilder`); those are the receiver of
/// the next chain hop, so their members must also be admitted. The closure
/// follows member return / declared / parameter types from external type to
/// external type until no new type is reached. It stays within the reachable
/// component — never the whole dependency tree — so the admitted write set is
/// bounded by `|reached types| × methods`, the same bound the seed has.
///
/// The set is empty when no external file is present (the common single-language
/// build), so the caller's second admission pass is a no-op there.
fn reachable_external_types(
    parsed: &[ParsedFile],
    arena: &TypeArena,
    supertypes: &SupertypeGraph,
) -> FxHashSet<String> {
    // Every external type's bare qname. A child / referenced type is "external"
    // iff it is declared in an `ext:` file, so this set is the membership test
    // for both the seed and the closure.
    let mut ext_type_qnames: FxHashSet<String> = FxHashSet::default();
    // For each external type, the external types its members reference through
    // a return / declared / parameter type — the closure's adjacency.
    let mut ext_member_edges: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();
    for pf in parsed {
        if !pf.path.starts_with("ext:") {
            continue;
        }
        for sym in &pf.symbols {
            if is_type_defining_kind(sym.kind) {
                ext_type_qnames.insert(strip_generic_args(&sym.qualified_name));
            }
        }
    }

    let mut reachable: FxHashSet<String> = FxHashSet::default();

    // Seed 1: external parents of internal → external supertype edges. The
    // child is internal exactly when its bare qname is not an external type.
    for (child, parent) in supertypes.child_parent_pairs() {
        let parent_qname = base_class_qname(parent, arena);
        let Some(parent_qname) = parent_qname else {
            continue;
        };
        if !ext_type_qnames.contains(parent_qname.as_str()) {
            continue;
        }
        let child_qname = base_class_qname(child, arena);
        let child_external = child_qname
            .as_deref()
            .is_some_and(|q| ext_type_qnames.contains(q));
        if !child_external {
            reachable.insert(parent_qname);
        }
    }

    // Seed 2: external types named by an internal symbol's type metadata, and
    // collect each external type's member-edge adjacency for the closure.
    for pf in parsed {
        let is_external = pf.path.starts_with("ext:");
        for sym in &pf.symbols {
            let referenced = symbol_referenced_types(sym, arena, &ext_type_qnames);
            if is_external {
                if let Some(scope) = &sym.scope_path {
                    let owner = strip_generic_args(scope);
                    if ext_type_qnames.contains(owner.as_str()) && !referenced.is_empty() {
                        ext_member_edges.entry(owner).or_default().extend(referenced);
                    }
                }
            } else {
                reachable.extend(referenced);
            }
        }
    }

    // Closure: walk member-type edges between external types from the seed.
    let mut frontier: Vec<String> = reachable.iter().cloned().collect();
    while let Some(ty) = frontier.pop() {
        let Some(next) = ext_member_edges.get(&ty) else {
            continue;
        };
        for n in next {
            if reachable.insert(n.clone()) {
                frontier.push(n.clone());
            }
        }
    }

    reachable
}

/// The external type qnames (generics-stripped) named by `sym`'s declared,
/// return, and parameter types, keeping only those in `ext_type_qnames`. Each
/// type is decomposed to its base class qname (`Repository<User>` →
/// `Repository`), so a generic application over an external base is recognized.
fn symbol_referenced_types(
    sym: &crate::types::ExtractedSymbol,
    arena: &TypeArena,
    ext_type_qnames: &FxHashSet<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut consider = |id: Option<TypeId>| {
        if let Some(id) = id {
            if let Some(q) = base_class_qname(id, arena) {
                if ext_type_qnames.contains(q.as_str()) {
                    out.push(q);
                }
            }
        }
    };
    consider(sym.declared_type);
    consider(sym.return_type);
    for &p in &sym.param_types {
        consider(Some(p));
    }
    out
}

/// The base nominal qname of a type, generics dropped, or `None` for a type
/// with no nominal base (primitive, function, tuple, literal, unknown). Peels
/// `Apply` to its base and the single-payload wrappers (`Optional`,
/// `AsyncWrapper`, `Iterator`) to their inner type so an external class wrapped
/// in `Promise<…>` / `…?` / `Iterator<…>` is still recognized.
fn base_class_qname(id: TypeId, arena: &TypeArena) -> Option<String> {
    match arena.get(id) {
        Type::Class(q) => Some(strip_generic_args(&q)),
        Type::Apply { base, .. } => base_class_qname(base, arena),
        Type::Optional(inner) | Type::AsyncWrapper(inner) | Type::Iterator(inner) => {
            base_class_qname(inner, arena)
        }
        _ => None,
    }
}

/// Symbol kinds that declare a nominal type with a member set — the owners
/// admitted by the reachability-bounded external pass.
fn is_type_defining_kind(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Class
            | SymbolKind::Struct
            | SymbolKind::Interface
            | SymbolKind::Trait
            | SymbolKind::Enum
            | SymbolKind::TypeAlias
    )
}

/// True when the language declares an extension function with the receiver
/// folded into the signature as a leading `this <Recv>` parameter — the shape
/// `this_extension_target` parses. C# extension methods
/// (`static R M(this T self, ...)`) and Kotlin extension functions
/// (`fun T.m()`, whose receiver the extractor lowers to a leading `this T`
/// parameter) both use it. Other languages do not, so their signatures are
/// never scanned for it.
fn extension_receiver_in_signature(language: &str) -> bool {
    matches!(language, "csharp" | "kotlin")
}

/// Returns the extended type when a method/function signature describes an
/// extension by folding its receiver into a leading `this <Type>` parameter:
/// `<ret> Name(this <Type> self, ...)`. The first parameter must use the
/// `this` modifier; the type identifier is taken up to the next whitespace,
/// generic bracket, or comma. Returns `None` for non-extension signatures.
/// Caller restricts to languages using this convention via
/// `extension_receiver_in_signature`.
fn this_extension_target(signature: &str) -> Option<&str> {
    let open = signature.find('(')?;
    let body = signature[open + 1..].trim_start();
    let rest = body.strip_prefix("this ")?.trim_start();
    let end = rest.find(|c: char| c.is_whitespace() || c == '<' || c == ',' || c == ')')?;
    let target = &rest[..end];
    if target.is_empty() {
        None
    } else {
        Some(target)
    }
}

#[cfg(test)]
#[path = "members_tests.rs"]
mod tests;
