// =============================================================================
// type_checker/core/members.rs — direct + extension member lookup
//
// MembersIndex is built once per indexing run from ParsedFiles. It owns two
// maps keyed by TypeId: `direct` (members declared inside the type's body —
// methods, fields, properties, enum members) and `extensions` (extension
// members declared outside the type — C# extension methods, Rust `impl T for
// U` blocks, Ruby class reopens). The chain walker calls `lookup` at every
// segment.
//
// Spec: research/architecture/02-engine-internal-architecture.html § Layer 3
//       research/architecture/04-implementation-phases.html § Phase 3
// =============================================================================

use super::types::{Type, TypeArena, TypeId};
use crate::indexer::canonical_form::signature_arity;
use crate::indexer::resolve::engine::{strip_generic_args, SymbolInfo};
use crate::type_checker::core::supertype::SupertypeGraph;
use crate::type_checker::profile::language_profile::{
    KindCompatibility, LanguageProfile,
};
use crate::types::{EdgeKind, ParsedFile, SymbolKind};
use rustc_hash::FxHashMap;
use std::str::FromStr;
use std::sync::Arc;

/// (file_path, parsed_file_symbol_index) → durable DB symbol id. Same shape as
/// `SymbolTypeMap::SymbolIdMap` so a single map can drive both builders.
pub type SymbolIdMap = FxHashMap<(String, usize), i64>;

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
        for pf in parsed {
            // External files (ext: prefix) carry thousands of symbols per
            // dep — for ts-nextjs that's ~1M symbols. Engine chain walks
            // resolve *into* internal types; external symbols stay
            // lookup-only via SymbolIndex. Skipping them at build time
            // turns a huge arena.class write storm into nothing.
            if pf.path.starts_with("ext:") {
                continue;
            }
            let file_path: Arc<str> = Arc::from(pf.path.as_str());
            for (idx, sym) in pf.symbols.iter().enumerate() {
                let Some(scope) = &sym.scope_path else {
                    continue;
                };
                if scope.is_empty() {
                    continue;
                }
                let Some(&sym_id) = sym_id_map.get(&(pf.path.clone(), idx)) else {
                    continue;
                };
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
                let bare_parent_ty = (bare_scope.as_str() != scope.as_str())
                    .then(|| arena.class(&bare_scope));
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
                let ext_target = if pf.language == "csharp" {
                    sym.signature
                        .as_deref()
                        .and_then(csharp_extension_target)
                } else {
                    None
                };
                if let Some(ext_type) = ext_target {
                    let ext_ty = arena.class(ext_type);
                    index.extensions.entry(ext_ty).or_default().push(info.clone());
                }
                if let Some(bare_ty) = bare_parent_ty {
                    index.direct.entry(bare_ty).or_default().push(info.clone());
                }
                index.direct.entry(parent_ty).or_default().push(info);
            }
        }
        index
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
        self.lookup_with_binding(ty, name, kind_filter, supertypes, arena, profile, None)
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
    ) -> Option<(SymbolInfo, TypeId, Vec<TypeId>)> {
        match arena.get(ty) {
            Type::Apply { base, .. } => {
                self.lookup_with_binding(base, name, kind_filter, supertypes, arena, profile, arg_count)
            }
            Type::Union(branches) => {
                // Every branch must carry the member — partial union members
                // are unsafe to resolve since the runtime value could land
                // on a branch missing the member. Returns the first branch's
                // match; the walker does not yet select a branch by a guard.
                let mut first: Option<(SymbolInfo, TypeId, Vec<TypeId>)> = None;
                for b in branches {
                    match self.lookup_with_binding(b, name, kind_filter, supertypes, arena, profile, arg_count) {
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
                    if let Some(s) =
                        self.lookup_with_binding(b, name, kind_filter, supertypes, arena, profile, arg_count)
                    {
                        return Some(s);
                    }
                }
                None
            }
            Type::Optional(inner) if profile.look_through_optional => {
                self.lookup_with_binding(inner, name, kind_filter, supertypes, arena, profile, arg_count)
            }
            Type::AsyncWrapper(inner) => {
                self.lookup_with_binding(inner, name, kind_filter, supertypes, arena, profile, arg_count)
            }
            Type::Iterator(inner) => {
                self.lookup_with_binding(inner, name, kind_filter, supertypes, arena, profile, arg_count)
            }
            Type::Class(_) | Type::Primitive(_) => {
                self.find_on_chain(ty, name, kind_filter, supertypes, arena, profile, arg_count)
            }
            // A bare generic parameter carries members only through its
            // declared upper bound: `T: Animal` resolves `T`'s members on
            // Animal. Recursion terminates because a bound is a Class/Apply
            // in every realistic declaration; an unbounded `T` has no members.
            Type::Generic { param } => match arena.generic_param(param).bound {
                Some(bound) => {
                    self.lookup_with_binding(bound, name, kind_filter, supertypes, arena, profile, arg_count)
                }
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
    /// arguments. Selection stays within one ancestor so inheritance overrides
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
    ) -> Option<(SymbolInfo, TypeId, Vec<TypeId>)> {
        for (ancestor, args) in supertypes.walk_up_with_args(ty, arena) {
            let mut first: Option<&SymbolInfo> = None;
            let mut arity_hit: Option<&SymbolInfo> = None;
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
                if arity_hit.is_none()
                    && arg_count.is_some()
                    && s.signature.as_deref().and_then(signature_arity) == arg_count
                {
                    arity_hit = Some(s);
                }
            }
            if let Some(found) = arity_hit.or(first) {
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

/// Returns the extended type when a C# method signature describes an extension
/// method: `<ret> Name(this <Type> self, ...)`. The first parameter must use
/// the `this` modifier; the type identifier is taken up to the next
/// whitespace, generic bracket, or comma. Returns `None` for non-extension
/// signatures. Caller is responsible for restricting to C# files.
fn csharp_extension_target(signature: &str) -> Option<&str> {
    let open = signature.find('(')?;
    let body = signature[open + 1..].trim_start();
    let rest = body.strip_prefix("this ")?.trim_start();
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '<' || c == ',' || c == ')')?;
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
