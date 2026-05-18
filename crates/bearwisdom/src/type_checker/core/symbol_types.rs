// =============================================================================
// type_checker/core/symbol_types.rs — per-symbol type metadata
//
// Replaces the scattered string-keyed maps on SymbolIndex
// (field_type / return_type / param_types) with one map keyed by the DB
// symbol id. Built once per indexing run from canonical-form ExtractedSymbol
// fields after symbols are persisted.
//
// Spec: research/architecture/02-engine-internal-architecture.html § Layer 1
// =============================================================================

use super::types::{GenericParamId, TypeArena, TypeId};
use crate::type_checker::profile::language_profile::LanguageProfile;
use crate::types::{ParsedFile, SymbolKind};
use rustc_hash::FxHashMap;

/// Maps `(file_path, parsed_file_symbol_index)` → durable DB symbol id. Built
/// by the indexer after symbols are persisted; consumed by
/// `SymbolTypeMap::build_from_parsed_files` so the type map keys by stable
/// id rather than per-run vector index.
pub type SymbolIdMap = FxHashMap<(String, usize), i64>;

/// Per-symbol type bundle. Every field is optional / empty when the symbol
/// kind does not carry it (e.g. a Class has return_type = Some(self_id) but
/// declared_type = None; a Method has param_types populated and
/// declared_type = None).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolTypeData {
    /// Annotated or inferred type for variables, fields, properties,
    /// parameters.
    pub declared_type: Option<TypeId>,
    /// Return type for functions, methods, constructors. For Class /
    /// Struct / Interface / Enum / TypeAlias symbols this is the symbol's
    /// own TypeId (classes-as-callable yield themselves).
    pub return_type: Option<TypeId>,
    /// Positional parameter types for callables. Empty for non-callable
    /// kinds.
    pub param_types: Vec<TypeId>,
    /// Generic parameter slots bound by this symbol (function, method, type).
    pub generic_params: Vec<GenericParamId>,
}

impl SymbolTypeData {
    pub fn is_empty(&self) -> bool {
        self.declared_type.is_none()
            && self.return_type.is_none()
            && self.param_types.is_empty()
            && self.generic_params.is_empty()
    }
}

/// Symbol id → SymbolTypeData lookup. The DB-side `symbols.id` is the key
/// because TypeIds and ExtractedSymbol indices are not durable across
/// indexing runs; symbol id is.
///
/// A parallel TypeId → sym_id index records the *self-yielding* mapping:
/// when a type-defining symbol's `return_type` equals `arena.class(qname)`,
/// this index lets the chain walker recover the symbol's generic params and
/// other metadata from a TypeId encountered mid-chain (typically the `base`
/// of a `Type::Apply`).
#[derive(Debug, Default)]
pub struct SymbolTypeMap {
    by_symbol_id: FxHashMap<i64, SymbolTypeData>,
    self_yielding: FxHashMap<TypeId, i64>,
}

impl SymbolTypeMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self {
            by_symbol_id: FxHashMap::with_capacity_and_hasher(cap, Default::default()),
            self_yielding: FxHashMap::default(),
        }
    }

    /// Look up the SymbolTypeData for the type-defining symbol whose
    /// self-yield equals `ty`. Lets the chain walker recover generic
    /// parameters or members from a Class TypeId without an O(n) scan.
    pub fn data_for_class(&self, ty: TypeId) -> Option<&SymbolTypeData> {
        self.self_yielding
            .get(&ty)
            .and_then(|sid| self.by_symbol_id.get(sid))
    }

    /// sym_id of the symbol whose self-yield TypeId is `ty`. None when no
    /// type-defining symbol claims this TypeId.
    pub fn sym_id_for_class(&self, ty: TypeId) -> Option<i64> {
        self.self_yielding.get(&ty).copied()
    }

    /// Number of symbols with non-empty type metadata recorded.
    pub fn len(&self) -> usize {
        self.by_symbol_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_symbol_id.is_empty()
    }

    /// Read the type bundle for a symbol id, if recorded.
    pub fn get(&self, sym_id: i64) -> Option<&SymbolTypeData> {
        self.by_symbol_id.get(&sym_id)
    }

    /// Insert / overwrite the bundle for `sym_id`. Empty bundles are
    /// discarded so `get` returns None for "no type info recorded."
    pub fn insert(&mut self, sym_id: i64, data: SymbolTypeData) {
        if data.is_empty() {
            // Drop the stale self-yielding entry too if there was one.
            if let Some(prev) = self.by_symbol_id.remove(&sym_id) {
                if let Some(prev_ty) = prev.return_type {
                    if self.self_yielding.get(&prev_ty) == Some(&sym_id) {
                        self.self_yielding.remove(&prev_ty);
                    }
                }
            }
            return;
        }
        self.by_symbol_id.insert(sym_id, data);
    }

    /// Mark `ty` as the self-yielding TypeId of `sym_id`. Used by the
    /// builder to register type-defining symbols in the reverse index.
    /// Idempotent — overwrites any prior owner of `ty`.
    pub fn mark_self_yielding(&mut self, ty: TypeId, sym_id: i64) {
        self.self_yielding.insert(ty, sym_id);
    }

    /// Mutable handle for incremental population (constructors-yield-self
    /// pass, decorator member synthesis, etc.). Creates an empty bundle on
    /// first access.
    pub fn entry(&mut self, sym_id: i64) -> &mut SymbolTypeData {
        self.by_symbol_id.entry(sym_id).or_default()
    }

    /// Read-only iterator over `(sym_id, data)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (i64, &SymbolTypeData)> + '_ {
        self.by_symbol_id.iter().map(|(k, v)| (*k, v))
    }

    /// Construct a SymbolTypeMap from per-file extraction output.
    ///
    /// Every type-defining symbol (Class, Struct, Interface, Trait, Enum,
    /// TypeAlias, Delegate) receives `return_type = Some(self_type_id)` —
    /// the canonical "callable type yields itself" rule. The arena is
    /// extended with the corresponding Class type for each such symbol.
    ///
    /// Symbols whose extractor populated the canonical TypeId-bearing
    /// fields directly on ExtractedSymbol (`declared_type`, `return_type`,
    /// `param_types`, `generic_params`) carry those values straight into
    /// the SymbolTypeData. Per-language extractor migration (Phase 5+)
    /// gradually populates these; SymbolTypeMap honours whatever the
    /// extractor surfaced, falling back to the self-yield rule only when
    /// `return_type` is None and the kind is type-defining.
    pub fn build_from_parsed_files(
        parsed: &[ParsedFile],
        sym_id_map: &SymbolIdMap,
        arena: &TypeArena,
        _profile: &LanguageProfile,
    ) -> Self {
        let mut map = SymbolTypeMap::new();
        for pf in parsed {
            for (idx, sym) in pf.symbols.iter().enumerate() {
                let Some(&sym_id) = sym_id_map.get(&(pf.path.clone(), idx)) else {
                    continue;
                };

                let extractor_return = sym.return_type;
                let extractor_declared = sym.declared_type;
                let extractor_params = sym.param_types.clone();
                let extractor_generics = sym.generic_params.clone();

                // Self-yield rule applies only when the extractor didn't
                // already produce a return_type. Lets per-language code
                // override the default for symbols where "callable yields
                // self" isn't the right answer (e.g. a future Python
                // metaclass profile that wants the metaclass instance).
                let (return_type, is_self_yield) = match (extractor_return, is_type_defining(sym.kind)) {
                    (Some(ty), defining) => {
                        // Honour extractor's choice. If it happens to match
                        // arena.class(qname), it's still a self-yield —
                        // mark accordingly so the reverse index works.
                        let self_yield = defining
                            && arena.class_lookup(&sym.qualified_name) == Some(ty);
                        (Some(ty), self_yield)
                    }
                    (None, true) => (Some(arena.class(&sym.qualified_name)), true),
                    (None, false) => (None, false),
                };

                let data = SymbolTypeData {
                    declared_type: extractor_declared,
                    return_type,
                    param_types: extractor_params,
                    generic_params: extractor_generics,
                };

                if !data.is_empty() {
                    map.insert(sym_id, data);
                    if is_self_yield {
                        if let Some(ty) = return_type {
                            map.mark_self_yielding(ty, sym_id);
                        }
                    }
                }
            }
        }
        map
    }
}

fn is_type_defining(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Class
            | SymbolKind::Struct
            | SymbolKind::Interface
            | SymbolKind::Trait
            | SymbolKind::Enum
            | SymbolKind::TypeAlias
            | SymbolKind::Delegate
    )
}

#[cfg(test)]
#[path = "symbol_types_tests.rs"]
mod tests;
