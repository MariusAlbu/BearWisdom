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
use crate::indexer::resolve::engine::SymbolInfo;
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
    /// - `Function` / `Tuple` / `Literal` / `Generic` / `Unknown`: no
    ///   members.
    pub fn lookup(
        &self,
        ty: TypeId,
        name: &str,
        kind_filter: EdgeKind,
        supertypes: &SupertypeGraph,
        arena: &TypeArena,
        profile: &LanguageProfile,
    ) -> Option<SymbolInfo> {
        match arena.get(ty) {
            Type::Apply { base, .. } => {
                self.lookup(base, name, kind_filter, supertypes, arena, profile)
            }
            Type::Union(branches) => {
                // Every branch must carry the member — partial union members
                // are unsafe to resolve since the runtime value could land
                // on a branch missing the member. First branch's match is
                // the returned symbol; chain walker narrows further if it
                // can.
                let mut first: Option<SymbolInfo> = None;
                for b in branches {
                    match self.lookup(b, name, kind_filter, supertypes, arena, profile) {
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
                        self.lookup(b, name, kind_filter, supertypes, arena, profile)
                    {
                        return Some(s);
                    }
                }
                None
            }
            Type::Optional(inner) if profile.look_through_optional => {
                self.lookup(inner, name, kind_filter, supertypes, arena, profile)
            }
            Type::AsyncWrapper(inner) => {
                self.lookup(inner, name, kind_filter, supertypes, arena, profile)
            }
            Type::Iterator(inner) => {
                self.lookup(inner, name, kind_filter, supertypes, arena, profile)
            }
            Type::Class(_) | Type::Primitive(_) => self.find_on_chain(
                ty, name, kind_filter, supertypes, profile,
            ),
            Type::Function { .. }
            | Type::Tuple(_)
            | Type::Literal(_)
            | Type::Generic { .. }
            | Type::Optional(_)
            | Type::Unknown => None,
        }
    }

    /// Walk the supertype chain starting at `ty` and find the first
    /// kind-compatible member named `name`. Direct members win over
    /// extension members at the same supertype level — extensions extend
    /// but do not override the body.
    fn find_on_chain(
        &self,
        ty: TypeId,
        name: &str,
        kind_filter: EdgeKind,
        supertypes: &SupertypeGraph,
        profile: &LanguageProfile,
    ) -> Option<SymbolInfo> {
        for ancestor in supertypes.walk_up(ty) {
            if let Some(found) = self
                .direct_of(ancestor)
                .iter()
                .chain(self.extensions_of(ancestor).iter())
                .find(|s| s.name == name && kind_matches(profile, kind_filter, &s.kind))
            {
                return Some(found.clone());
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

#[cfg(test)]
#[path = "members_tests.rs"]
mod tests;
